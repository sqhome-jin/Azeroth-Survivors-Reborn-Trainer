import Foundation

struct SettingsDTO: Codable, Equatable {
    var gamePath: String?
    var language: String
    var appearance: String
    var enabledStats: [String]
    var targets: [String: UInt32]
    var recoveryNotice: String?
}

struct StatMeta: Codable, Identifiable, Hashable {
    var key: String
    var label: String
    var min: UInt32
    var max: UInt32
    var percent: Bool
    var defaultEnabled: Bool

    var id: String { key }
}

struct GameSession: Codable, Identifiable, Hashable {
    var sessionId: UInt64
    var pid: UInt32
    var created: UInt64
    var path: String

    var id: UInt64 { sessionId }
}

struct DiscoverResult: Codable {
    var sessions: [GameSession]
    var inaccessibleCount: Int
    var preferredSessionId: UInt64?
    var running: Bool
}

struct CharacterDTO: Codable {
    var name: String
    var count: Int
    var index: Int
    var healthCandidate: [Int]
}

struct SnapshotDTO: Codable {
    var build: String
    var character: CharacterDTO?
    var statLayout: String
    var mappedStats: [String]
    var values: [String: UInt64]
    var proofId: UInt64?
}

struct ApplyOutcomeDTO: Codable {
    var values: [String: UInt64]
    var applied: [String]
    var skipped: [String: String]
}

struct UpdateStatusDTO: Codable {
    var tag: String
    var newer: Bool
    var releaseUrl: String
    var assetUrl: String?
}

struct DiagnosticsPaths: Codable {
    var stateDirectory: String
    var settingsPath: String
    var logPath: String
    var previousLogPath: String
    var platform: String
    var version: String
}

struct BackendEvent: Codable {
    var kind: String
    var jobId: UInt64?
    var operation: String?
    var code: String?
    var message: String?
    var payload: JSONValue
}

enum JSONValue: Codable, Equatable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else if let value = try? container.decode([String: JSONValue].self) {
            self = .object(value)
        } else {
            throw DecodingError.dataCorruptedError(in: container, debugDescription: "Unsupported JSON")
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case .bool(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .string(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        }
    }

    func data() throws -> Data {
        try JSONSerialization.data(withJSONObject: asAny())
    }

    func asAny() -> Any {
        switch self {
        case .null: return NSNull()
        case .bool(let value): return value
        case .number(let value):
            if value.rounded() == value, value >= Double(Int64.min), value <= Double(Int64.max) {
                return Int64(value)
            }
            return value
        case .string(let value): return value
        case .array(let value): return value.map { $0.asAny() }
        case .object(let value): return value.mapValues { $0.asAny() }
        }
    }
}

enum SidebarItem: String, CaseIterable, Identifiable {
    case game
    case stats
    case diagnostics

    var id: String { rawValue }

    func title(chinese: Bool) -> String {
        switch self {
        case .game: return chinese ? "游戏" : "Game"
        case .stats: return chinese ? "属性" : "Stats"
        case .diagnostics: return chinese ? "诊断" : "Diagnostics"
        }
    }

    var symbol: String {
        switch self {
        case .game: return "gamecontroller"
        case .stats: return "slider.horizontal.3"
        case .diagnostics: return "stethoscope"
        }
    }
}
