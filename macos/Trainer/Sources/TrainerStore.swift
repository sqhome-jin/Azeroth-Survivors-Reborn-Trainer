import AppKit
import Foundation
import SwiftUI

@MainActor
final class TrainerStore: ObservableObject {
    @Published var sidebar: SidebarItem = .stats
    @Published var settings: SettingsDTO
    @Published var stats: [StatMeta] = []
    @Published var sessions: [GameSession] = []
    @Published var selectedSessionId: UInt64?
    @Published var inaccessibleCount = 0
    @Published var gameRunning = false
    @Published var selectedStatKey: String?
    @Published var draftTarget: String = ""
    @Published var values: [String: UInt64] = [:]
    @Published var mappedStats: Set<String> = []
    @Published var proofId: UInt64?
    @Published var characterName: String?
    @Published var buildId: String?
    @Published var statusMessage = ""
    @Published var busyOperation: String?
    @Published var updateStatus: UpdateStatusDTO?
    @Published var diagnostics: DiagnosticsPaths?
    @Published var logTail = ""
    @Published var confirmStop = false
    @Published var errorMessage: String?

    private let bridge: RustBridge
    private var pollTask: Task<Void, Never>?
    private var discoverTask: Task<Void, Never>?
    private var activeJobId: UInt64?
    private var applyAfterFind = false

    var chinese: Bool { settings.language.lowercased().hasPrefix("zh") }
    var connected: Bool { selectedSessionId != nil }
    var selectedStat: StatMeta? { stats.first { $0.key == selectedStatKey } }

    init() {
        let bridge: RustBridge
        do {
            bridge = try RustBridge()
        } catch {
            fatalError("Unable to start trainer backend: \(error)")
        }
        self.bridge = bridge
        do {
            settings = try bridge.loadSettings()
            if settings.gamePath == nil {
                settings.gamePath = try bridge.defaultGamePath()
            }
            stats = try bridge.statsMetadata(chinese: settings.language.lowercased().hasPrefix("zh"))
            selectedStatKey = stats.first?.key
            if let key = selectedStatKey {
                draftTarget = targetString(for: key)
            }
            diagnostics = try? bridge.diagnosticsPaths()
            statusMessage = settings.language.lowercased().hasPrefix("zh")
                ? "请启动游戏以查找角色。"
                : "Start Reborn to find a character."
            if let notice = settings.recoveryNotice {
                errorMessage = notice
            }
            applyAppearance()
            startPolling()
            refreshDiscovery()
            checkForUpdates()
        } catch {
            settings = SettingsDTO(
                gamePath: nil,
                language: "en",
                appearance: "system",
                enabledStats: ["damage", "cooldown", "area", "luck"],
                targets: [:],
                recoveryNotice: nil
            )
            errorMessage = error.localizedDescription
            startPolling()
        }
    }

    deinit {
        pollTask?.cancel()
        discoverTask?.cancel()
    }

    func t(_ en: String, _ zh: String) -> String { chinese ? zh : en }

    func persistSettings() {
        do { try bridge.saveSettings(settings) }
        catch { errorMessage = error.localizedDescription }
    }

    func setLanguage(_ code: String) {
        settings.language = code
        persistSettings()
        do { stats = try bridge.statsMetadata(chinese: chinese) }
        catch { errorMessage = error.localizedDescription }
    }

    func setAppearance(_ code: String) {
        settings.appearance = code
        persistSettings()
        applyAppearance()
    }

    private func applyAppearance() {
        switch settings.appearance {
        case "light": NSApp.appearance = NSAppearance(named: .aqua)
        case "dark": NSApp.appearance = NSAppearance(named: .darkAqua)
        default: NSApp.appearance = nil
        }
    }

    func isEnabled(_ key: String) -> Bool { settings.enabledStats.contains(key) }

    func toggleEnabled(_ key: String, enabled: Bool) {
        if enabled {
            if !settings.enabledStats.contains(key) { settings.enabledStats.append(key) }
        } else {
            settings.enabledStats.removeAll { $0 == key }
        }
        persistSettings()
        if enabled, proofId != nil, mappedStats.contains(key) {
            applyCheckedTargets(keys: [key])
        }
    }

    func target(for key: String) -> UInt32 {
        settings.targets[key] ?? stats.first { $0.key == key }?.max ?? 0
    }

    func targetString(for key: String) -> String {
        if key == "health" { return t("Full", "满血") }
        return String(target(for: key))
    }

    func selectStat(_ key: String) {
        commitDraftTarget()
        selectedStatKey = key
        draftTarget = targetString(for: key)
    }

    func commitDraftTarget() {
        guard let key = selectedStatKey, let stat = selectedStat, key != "health" else { return }
        guard let value = UInt32(draftTarget.trimmingCharacters(in: .whitespacesAndNewlines)),
              (stat.min...stat.max).contains(value)
        else {
            draftTarget = targetString(for: key)
            return
        }
        settings.targets[key] = value
        persistSettings()
    }

    func setMaxTarget() {
        guard let stat = selectedStat else { return }
        draftTarget = String(stat.max)
        settings.targets[stat.key] = stat.max
        persistSettings()
    }

    func locateGame() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.allowsMultipleSelection = false
        panel.title = t("Locate AzerothSurvivorsGame", "定位 AzerothSurvivorsGame")
        if let path = settings.gamePath {
            panel.directoryURL = URL(fileURLWithPath: path).deletingLastPathComponent()
        }
        guard panel.runModal() == .OK, let url = panel.url else { return }
        settings.gamePath = url.path
        persistSettings()
    }

    func runGame() {
        guard let path = settings.gamePath else {
            statusMessage = t("Locate the game binary first.", "请先定位游戏二进制文件。")
            return
        }
        do {
            activeJobId = try bridge.launch(path: path)
            busyOperation = "launch"
            statusMessage = t("Starting game…", "正在启动游戏…")
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func requestStop() { confirmStop = selectedSessionId != nil }

    func confirmStopGame() {
        confirmStop = false
        guard let sessionId = selectedSessionId else { return }
        do {
            activeJobId = try bridge.stop(sessionId: sessionId)
            busyOperation = "stop"
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func findStats(applyChecked: Bool) {
        commitDraftTarget()
        guard let sessionId = selectedSessionId else {
            statusMessage = t("Start Reborn to find a character.", "请启动游戏以查找角色。")
            return
        }
        applyAfterFind = applyChecked
        do {
            activeJobId = try bridge.find(sessionId: sessionId)
            busyOperation = "find"
            statusMessage = t("Finding character and stats…", "正在查找角色和属性…")
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func readValues() { findStats(applyChecked: false) }

    func applySelected() {
        commitDraftTarget()
        guard let key = selectedStatKey else { return }
        applyCheckedTargets(keys: [key])
    }

    func applyCheckedTargets(keys: [String]? = nil) {
        guard let proofId else {
            statusMessage = t("Find a character first.", "请先查找角色。")
            return
        }
        let keys = keys ?? settings.enabledStats.filter { mappedStats.contains($0) }
        var targets: [String: UInt32] = [:]
        for key in keys {
            targets[key] = key == "health" ? 0 : target(for: key)
        }
        guard !targets.isEmpty else {
            statusMessage = t("No mapped stats are enabled.", "没有可写入的已勾选属性。")
            return
        }
        do {
            activeJobId = try bridge.apply(proofId: proofId, targets: targets)
            busyOperation = "apply"
            statusMessage = t("Applying stats…", "正在应用属性…")
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func cancelBusy() {
        if let activeJobId { bridge.cancel(jobId: activeJobId) }
    }

    func checkForUpdates() {
        // Background update probes must not steal the Find status line or pop alerts.
        // No stable GitHub release yet is expected during development (404).
        do {
            let version = try bridge.version()
            activeJobId = try bridge.updateCheck(current: version)
        } catch {
            updateStatus = nil
        }
    }

    func openUpdate() {
        let fallback = "https://github.com/sqhome-jin/Azeroth-Survivors-Reborn-Trainer/releases"
        guard let url = URL(string: updateStatus?.releaseUrl ?? fallback) else { return }
        NSWorkspace.shared.open(url)
    }

    func refreshDiagnostics() {
        diagnostics = try? bridge.diagnosticsPaths()
        logTail = (try? bridge.recentLogLines()) ?? ""
    }

    func copyDiagnostics() {
        let text = """
        platform=\(diagnostics?.platform ?? "")
        version=\(diagnostics?.version ?? "")
        build=\(buildId ?? "")
        character=\(characterName ?? "")
        session=\(selectedSessionId.map(String.init) ?? "")
        mapped=\(mappedStats.sorted().joined(separator: ","))
        values=\(values)
        targets=\(settings.targets)
        log:
        \(logTail)
        """
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    func openLogFolder() {
        guard let path = diagnostics?.stateDirectory else { return }
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    private func refreshDiscovery() {
        do {
            let result = try bridge.discover()
            sessions = result.sessions
            inaccessibleCount = result.inaccessibleCount
            gameRunning = result.running
            let previous = selectedSessionId
            if let preferred = result.preferredSessionId,
               result.sessions.contains(where: { $0.sessionId == preferred })
            {
                selectedSessionId = preferred
            } else if let current = selectedSessionId,
                      result.sessions.contains(where: { $0.sessionId == current })
            {
                selectedSessionId = current
            } else {
                selectedSessionId = result.sessions.count == 1 ? result.sessions.first?.sessionId : nil
            }
            if previous != selectedSessionId { clearProof() }
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func clearProof() {
        if let proofId { bridge.releaseProof(proofId) }
        proofId = nil
        values = [:]
        mappedStats = []
        characterName = nil
        buildId = nil
    }

    private func startPolling() {
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                self?.drainEvents()
                try? await Task.sleep(nanoseconds: 100_000_000)
            }
        }
        discoverTask = Task { [weak self] in
            while !Task.isCancelled {
                await MainActor.run { self?.refreshDiscovery() }
                try? await Task.sleep(nanoseconds: 1_000_000_000)
            }
        }
    }

    private func drainEvents() {
        while let event = try? bridge.pollEvent() {
            handle(event: event)
        }
    }

    private func handle(event: BackendEvent) {
        switch event.kind {
        case "job_completed":
            let operation = event.operation
            activeJobId = nil
            if busyOperation == operation {
                busyOperation = nil
            }
            switch operation {
            case "find":
                if let snapshot = try? decodePayload(SnapshotDTO.self, from: event.payload) {
                    buildId = snapshot.build
                    characterName = snapshot.character?.name
                    values = snapshot.values
                    mappedStats = Set(snapshot.mappedStats)
                    if let old = proofId { bridge.releaseProof(old) }
                    proofId = snapshot.proofId
                    statusMessage = t("Character and stats found.", "已找到角色和属性。")
                    if applyAfterFind {
                        applyAfterFind = false
                        applyCheckedTargets()
                    }
                }
            case "apply":
                if let outcome = try? decodePayload(ApplyOutcomeDTO.self, from: event.payload) {
                    values.merge(outcome.values) { _, new in new }
                    statusMessage = t("Applied checked targets.", "已应用勾选目标。")
                }
            case "launch", "stop":
                refreshDiscovery()
                statusMessage = operation == "stop"
                    ? t("Game stopped.", "游戏已停止。")
                    : t("Game started.", "游戏已启动。")
            case "update_check":
                updateStatus = try? decodePayload(UpdateStatusDTO.self, from: event.payload)
            default:
                break
            }
        case "job_failed", "job_cancelled":
            let operation = event.operation
            activeJobId = nil
            if busyOperation == operation {
                busyOperation = nil
            }
            if operation == "update_check" {
                updateStatus = nil
                break
            }
            if event.kind == "job_cancelled" {
                if operation == "find" || operation == "apply" {
                    statusMessage = t("Search stopped.", "查找已停止。")
                }
            } else {
                errorMessage = event.message
                statusMessage = event.message ?? t("Operation failed.", "操作失败。")
            }
        default:
            break
        }
    }

    private func decodePayload<T: Decodable>(_ type: T.Type, from value: JSONValue) throws -> T {
        try JSONDecoder().decode(type, from: value.data())
    }
}
