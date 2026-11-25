import AppKit
import SwiftUI

struct RootView: View {
    @ObservedObject var store: TrainerStore

    var body: some View {
        NavigationSplitView {
            List(selection: $store.sidebar) {
                ForEach(SidebarItem.allCases) { item in
                    Label(item.title(chinese: store.chinese), systemImage: item.symbol)
                        .tag(item)
                }
            }
            .navigationSplitViewColumnWidth(min: 180, ideal: 200, max: 240)
            .safeAreaInset(edge: .bottom) {
                VStack(alignment: .leading, spacing: 8) {
                    Picker(store.t("Language", "语言"), selection: Binding(
                        get: { store.settings.language },
                        set: { store.setLanguage($0) }
                    )) {
                        Text("English").tag("en")
                        Text("简体中文").tag("zh-Hans")
                    }
                    Picker(store.t("Appearance", "外观"), selection: Binding(
                        get: { store.settings.appearance },
                        set: { store.setAppearance($0) }
                    )) {
                        Text(store.t("System", "跟随系统")).tag("system")
                        Text(store.t("Light", "浅色")).tag("light")
                        Text(store.t("Dark", "深色")).tag("dark")
                    }
                }
                .pickerStyle(.menu)
                .padding()
            }
        } detail: {
            switch store.sidebar {
            case .game:
                GameView(store: store)
            case .stats:
                StatsView(store: store)
            case .diagnostics:
                DiagnosticsView(store: store)
            }
        }
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                connectionBadge
                Button(store.t("Run", "运行"), action: store.runGame)
                Button(store.t("Stop", "停止"), action: store.requestStop)
                Button(store.busyOperation == "find" ? store.t("Stop Search", "停止查找") : store.t("Find", "查找")) {
                    if store.busyOperation == "find" {
                        store.cancelBusy()
                    } else {
                        store.findStats(applyChecked: true)
                    }
                }
                Button(store.t("Read", "读取"), action: store.readValues)
            }
        }
        .alert(store.t("Stop game?", "停止游戏？"), isPresented: $store.confirmStop) {
            Button(store.t("Stop", "停止"), role: .destructive, action: store.confirmStopGame)
            Button(store.t("Cancel", "取消"), role: .cancel) {}
        } message: {
            Text(store.sessions.first { $0.sessionId == store.selectedSessionId }?.path ?? "")
        }
        .alert(store.t("Error", "错误"), isPresented: Binding(
            get: { store.errorMessage != nil },
            set: { if !$0 { store.errorMessage = nil } }
        )) {
            Button("OK", role: .cancel) { store.errorMessage = nil }
        } message: {
            Text(store.errorMessage ?? "")
        }
        .overlay(alignment: .top) {
            if store.updateStatus?.newer == true {
                HStack {
                    Text(store.t("Update available", "有可用更新") + ": \(store.updateStatus?.tag ?? "")")
                    Spacer()
                    Button(store.t("GitHub", "GitHub"), action: store.openUpdate)
                }
                .padding(10)
                .background(.yellow.opacity(0.2))
            }
        }
    }

    private var connectionBadge: some View {
        Label {
            Text(store.connected
                ? store.t("Game connected", "游戏已连接")
                : store.gameRunning
                    ? store.t("Multiple game processes", "检测到多个游戏进程")
                    : store.t("Waiting for game", "等待游戏"))
        } icon: {
            Image(systemName: "circle.fill")
                .foregroundStyle(store.connected ? .green : .secondary)
        }
    }
}

struct GameView: View {
    @ObservedObject var store: TrainerStore

    var body: some View {
        Form {
            Section(store.t("Game", "游戏")) {
                LabeledContent(store.t("Executable", "可执行文件")) {
                    Text(store.settings.gamePath ?? store.t("Not set", "未设置"))
                        .lineLimit(2)
                        .textSelection(.enabled)
                }
                HStack {
                    Button(store.t("Locate…", "定位…"), action: store.locateGame)
                    Button(store.t("Run", "运行"), action: store.runGame)
                    Button(store.t("Stop", "停止"), action: store.requestStop)
                        .disabled(store.selectedSessionId == nil)
                }
            }
            Section(store.t("Sessions", "进程")) {
                if store.sessions.isEmpty {
                    Text(store.t("No game process detected.", "未检测到游戏进程。"))
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(store.sessions) { session in
                        LabeledContent("PID \(session.pid)") {
                            Text(session.path).lineLimit(1)
                        }
                    }
                }
                if store.inaccessibleCount > 0 {
                    Text(store.t(
                        "Some processes are inaccessible. Sign the app with the debugger entitlement.",
                        "部分进程无法访问。请确保应用已使用 debugger entitlement 签名。"
                    ))
                    .foregroundStyle(.secondary)
                }
            }
            Section(store.t("Status", "状态")) {
                Text(store.statusMessage)
                if let busy = store.busyOperation {
                    ProgressView(store.t("Working…", "处理中…") + " (\(busy))")
                }
            }
        }
        .formStyle(.grouped)
        .padding()
    }
}

struct StatsView: View {
    @ObservedObject var store: TrainerStore

    var body: some View {
        HSplitView {
            List(selection: Binding(
                get: { store.selectedStatKey },
                set: { if let key = $0 { store.selectStat(key) } }
            )) {
                ForEach(store.stats) { stat in
                    HStack {
                        Toggle("", isOn: Binding(
                            get: { store.isEnabled(stat.key) },
                            set: { store.toggleEnabled(stat.key, enabled: $0) }
                        ))
                        .labelsHidden()
                        .toggleStyle(.checkbox)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(stat.label)
                            Text(currentText(for: stat))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Text(store.targetString(for: stat.key))
                            .foregroundStyle(.secondary)
                    }
                    .tag(stat.key)
                }
            }
            .frame(minWidth: 320)

            Form {
                Section(store.t("Selected stat", "选中属性")) {
                    if let stat = store.selectedStat {
                        LabeledContent(store.t("Stat", "属性"), value: stat.label)
                        LabeledContent(store.t("Current", "当前"), value: currentText(for: stat))
                        if stat.key == "health" {
                            Text(store.t(
                                "Current health follows verified maximum health.",
                                "当前生命值跟随已验证的最大生命值。"
                            ))
                            .foregroundStyle(.secondary)
                        } else {
                            TextField(store.t("Target", "目标"), text: $store.draftTarget)
                                .onSubmit(store.commitDraftTarget)
                            HStack {
                                Button(store.t("Use maximum", "使用最大值"), action: store.setMaxTarget)
                                Button(store.t("Apply", "应用"), action: store.applySelected)
                                    .disabled(store.proofId == nil || store.busyOperation != nil)
                            }
                        }
                        if !store.mappedStats.contains(stat.key), store.proofId != nil {
                            Text(store.t(
                                "This stat is unavailable on the detected build.",
                                "当前游戏版本无法修改此属性。"
                            ))
                            .foregroundStyle(.secondary)
                        }
                    } else {
                        Text(store.t("Select a stat.", "请选择一个属性。"))
                    }
                }
                Section(store.t("Discovery", "查找")) {
                    Text(store.statusMessage)
                    if let name = store.characterName {
                        LabeledContent(store.t("Character", "角色"), value: name)
                    }
                    if let build = store.buildId {
                        LabeledContent(store.t("Build", "构建"), value: build)
                    }
                    Text(store.t("Find applies checked targets once.", "查找会应用一次已勾选的目标值。"))
                        .foregroundStyle(.secondary)
                }
            }
            .formStyle(.grouped)
            .frame(minWidth: 280)
        }
        .padding()
    }

    private func currentText(for stat: StatMeta) -> String {
        guard let value = store.values[stat.key] else {
            return store.mappedStats.contains(stat.key) || store.proofId == nil ? "—" : store.t("Unavailable", "不可用")
        }
        let number = formatted(value)
        return stat.percent ? "\(number)%" : number
    }

    private func formatted(_ value: UInt64) -> String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        return formatter.string(from: NSNumber(value: value)) ?? String(value)
    }
}

struct DiagnosticsView: View {
    @ObservedObject var store: TrainerStore

    var body: some View {
        Form {
            Section(store.t("Paths", "路径")) {
                LabeledContent("Platform", value: store.diagnostics?.platform ?? "—")
                LabeledContent("Version", value: store.diagnostics?.version ?? "—")
                LabeledContent("State", value: store.diagnostics?.stateDirectory ?? "—")
                LabeledContent("Log", value: store.diagnostics?.logPath ?? "—")
                HStack {
                    Button(store.t("Refresh", "刷新"), action: store.refreshDiagnostics)
                    Button(store.t("Copy details", "复制详情"), action: store.copyDiagnostics)
                    Button(store.t("Open folder", "打开目录"), action: store.openLogFolder)
                    Button(store.t("Check updates", "检查更新"), action: store.checkForUpdates)
                }
            }
            Section(store.t("Recent log", "最近日志")) {
                ScrollView {
                    Text(store.logTail.isEmpty ? store.t("No log yet.", "暂无日志。") : store.logTail)
                        .font(.system(.caption, design: .monospaced))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .textSelection(.enabled)
                }
                .frame(minHeight: 280)
            }
        }
        .formStyle(.grouped)
        .padding()
        .onAppear(perform: store.refreshDiagnostics)
    }
}
