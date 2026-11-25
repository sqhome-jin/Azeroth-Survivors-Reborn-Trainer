import Foundation

enum BridgeError: LocalizedError {
    case backendUnavailable
    case status(Int32)
    case message(String)

    var errorDescription: String? {
        switch self {
        case .backendUnavailable: return "Trainer backend is unavailable."
        case .status(let code): return "Backend error (\(code))."
        case .message(let text): return text
        }
    }
}

final class RustBridge: @unchecked Sendable {
    private var handle: OpaquePointer?

    init() throws {
        guard let created = rt_backend_create() else {
            throw BridgeError.backendUnavailable
        }
        handle = created
    }

    deinit {
        if let handle {
            rt_backend_destroy(handle)
        }
    }

    private func requireHandle() throws -> OpaquePointer {
        guard let handle else { throw BridgeError.backendUnavailable }
        return handle
    }

    private func takeBuffer(_ buffer: inout RtBuffer) -> Data {
        defer { rt_buffer_free(buffer) }
        guard let data = buffer.data, buffer.len > 0 else { return Data() }
        return Data(bytes: data, count: Int(buffer.len))
    }

    private func decode<T: Decodable>(_ type: T.Type, from buffer: inout RtBuffer) throws -> T {
        try JSONDecoder().decode(type, from: takeBuffer(&buffer))
    }

    private func text(from buffer: inout RtBuffer) -> String {
        String(data: takeBuffer(&buffer), encoding: .utf8) ?? ""
    }

    func version() throws -> String {
        var buffer = RtBuffer()
        let status = rt_version(try requireHandle(), &buffer)
        guard status == RT_OK else { throw BridgeError.status(status) }
        return text(from: &buffer)
    }

    func loadSettings() throws -> SettingsDTO {
        var buffer = RtBuffer()
        let status = rt_settings_load(try requireHandle(), &buffer)
        guard status == RT_OK else { throw BridgeError.status(status) }
        return try decode(SettingsDTO.self, from: &buffer)
    }

    func saveSettings(_ settings: SettingsDTO) throws {
        let data = try JSONEncoder().encode(settings)
        var out = RtBuffer()
        let handle = try requireHandle()
        let code = data.withUnsafeBytes { pointer in
            rt_settings_save(
                handle,
                pointer.bindMemory(to: UInt8.self).baseAddress,
                data.count,
                &out
            )
        }
        defer { rt_buffer_free(out) }
        guard code == RT_OK else {
            let message = text(from: &out)
            throw message.isEmpty ? BridgeError.status(code) : BridgeError.message(message)
        }
    }

    func defaultGamePath() throws -> String? {
        var buffer = RtBuffer()
        let status = rt_process_default_path(try requireHandle(), &buffer)
        if status == RT_EMPTY { return nil }
        guard status == RT_OK else { throw BridgeError.status(status) }
        return text(from: &buffer)
    }

    func discover() throws -> DiscoverResult {
        var buffer = RtBuffer()
        let status = rt_process_discover(try requireHandle(), &buffer)
        guard status == RT_OK else { throw BridgeError.status(status) }
        return try decode(DiscoverResult.self, from: &buffer)
    }

    func launch(path: String) throws -> UInt64 {
        var job: UInt64 = 0
        let handle = try requireHandle()
        let code = path.withCString { rt_process_launch(handle, $0, &job) }
        guard code == RT_OK else { throw BridgeError.status(code) }
        return job
    }

    func stop(sessionId: UInt64) throws -> UInt64 {
        var job: UInt64 = 0
        let code = rt_process_stop(try requireHandle(), sessionId, &job)
        guard code == RT_OK else { throw BridgeError.status(code) }
        return job
    }

    func find(sessionId: UInt64) throws -> UInt64 {
        var job: UInt64 = 0
        let code = rt_hunt_find(try requireHandle(), sessionId, &job)
        guard code == RT_OK else { throw BridgeError.status(code) }
        return job
    }

    func apply(proofId: UInt64, targets: [String: UInt32]) throws -> UInt64 {
        let data = try JSONEncoder().encode(targets)
        var job: UInt64 = 0
        let handle = try requireHandle()
        let code = data.withUnsafeBytes { pointer in
            rt_hunt_apply(
                handle,
                proofId,
                pointer.bindMemory(to: UInt8.self).baseAddress,
                data.count,
                &job
            )
        }
        guard code == RT_OK else { throw BridgeError.status(code) }
        return job
    }

    func cancel(jobId: UInt64) {
        guard let handle else { return }
        _ = rt_job_cancel(handle, jobId)
    }

    func releaseProof(_ proofId: UInt64) {
        guard let handle else { return }
        _ = rt_proof_release(handle, proofId)
    }

    func updateCheck(current: String) throws -> UInt64 {
        var job: UInt64 = 0
        let handle = try requireHandle()
        let code = current.withCString { rt_update_check(handle, $0, &job) }
        guard code == RT_OK else { throw BridgeError.status(code) }
        return job
    }

    func diagnosticsPaths() throws -> DiagnosticsPaths {
        var buffer = RtBuffer()
        let status = rt_diagnostics_paths(try requireHandle(), &buffer)
        guard status == RT_OK else { throw BridgeError.status(status) }
        return try decode(DiagnosticsPaths.self, from: &buffer)
    }

    func recentLogLines() throws -> String {
        var buffer = RtBuffer()
        let status = rt_diagnostics_recent_lines(try requireHandle(), &buffer)
        guard status == RT_OK else { throw BridgeError.status(status) }
        return text(from: &buffer)
    }

    func statsMetadata(chinese: Bool) throws -> [StatMeta] {
        var buffer = RtBuffer()
        let status = rt_stats_metadata(try requireHandle(), chinese ? 1 : 0, &buffer)
        guard status == RT_OK else { throw BridgeError.status(status) }
        return try decode([StatMeta].self, from: &buffer)
    }

    func pollEvent() throws -> BackendEvent? {
        var buffer = RtBuffer()
        let status = rt_poll_event(try requireHandle(), &buffer)
        if status == RT_EMPTY { return nil }
        guard status == RT_OK else { throw BridgeError.status(status) }
        return try decode(BackendEvent.self, from: &buffer)
    }
}
