// Mirrors the daemon's JSON protocol (crates/fan-core/src/protocol.rs).
// Keys arrive in snake_case; decode with `JSONDecoder.daemon`.

import Foundation

public enum Mode: Equatable, Sendable {
    case system
    case smart
    case fixed(rpm: Double)
    case max

    public var kind: String {
        switch self {
        case .system: "system"
        case .smart: "smart"
        case .fixed: "fixed"
        case .max: "max"
        }
    }
}

extension Mode: Codable {
    private enum CodingKeys: String, CodingKey { case kind, rpm }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .kind) {
        case "system": self = .system
        case "smart": self = .smart
        case "max": self = .max
        case "fixed": self = .fixed(rpm: try c.decode(Double.self, forKey: .rpm))
        case let other:
            throw DecodingError.dataCorruptedError(forKey: .kind, in: c, debugDescription: "unknown mode \(other)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(kind, forKey: .kind)
        if case let .fixed(rpm) = self { try c.encode(rpm, forKey: .rpm) }
    }
}

public enum Profile: String, Codable, CaseIterable, Sendable {
    case quiet, balanced, performance, custom

    public var title: String { rawValue.capitalized }
}

public enum Decision: Equatable, Sendable {
    case system
    case duty(pct: Double)
    case rpm(rpm: Double, floorPct: Double)
}

extension Decision: Decodable {
    private enum CodingKeys: String, CodingKey { case kind, pct, rpm, floorPct }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .kind) {
        case "duty": self = .duty(pct: try c.decode(Double.self, forKey: .pct))
        case "rpm":
            self = .rpm(
                rpm: try c.decode(Double.self, forKey: .rpm),
                floorPct: try c.decodeIfPresent(Double.self, forKey: .floorPct) ?? 0)
        default: self = .system
        }
    }
}

public enum Reason: String, Decodable, Sendable {
    case normal, idle, critical, protecting
    case sensorFailure = "sensor_failure"
    case sensorGlitch = "sensor_glitch"
    case controlDisabled = "control_disabled"

    public init(from decoder: Decoder) throws {
        // Unknown future reasons degrade gracefully instead of failing the whole status.
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = Reason(rawValue: raw) ?? .normal
    }

    public var explanation: String {
        switch self {
        case .normal: "Following the selected mode"
        case .idle: "Cool — macOS is in charge (fans may be off)"
        case .critical: "Critical temperature — fans at maximum"
        case .protecting: "Hot — fans raised above the fixed speed"
        case .sensorFailure: "Sensors unavailable — macOS is in charge"
        case .sensorGlitch: "Sensor read failed — holding last speed"
        case .controlDisabled: "Fans didn't respond — macOS is in charge"
        }
    }
}

public struct FanInfo: Decodable, Equatable, Sendable {
    public let index: Int
    public let minRpm: Double
    public let maxRpm: Double
}

public struct FanReading: Decodable, Equatable, Sendable {
    public let actualRpm: Double
    public let targetRpm: Double
    public let forced: Bool
}

public struct FanStatus: Decodable, Equatable, Sendable, Identifiable {
    public let info: FanInfo
    public let reading: FanReading?
    public let commandedRpm: Double?
    /// Runtime check that the fan obeys control: nil until tested.
    public let verified: Bool?

    public var id: Int { info.index }

    /// Actual speed as a fraction of max (0–1).
    public var fraction: Double {
        guard let r = reading, info.maxRpm > 0 else { return 0 }
        return min(max(r.actualRpm / info.maxRpm, 0), 1)
    }
}

public enum SupportLevel: String, Decodable, Sendable {
    case validated
    case compatible
    case monitorOnly = "monitor_only"

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = SupportLevel(rawValue: raw) ?? .monitorOnly
    }
}

public struct GroupSummary: Decodable, Equatable, Sendable, Identifiable {
    public let group: String
    public let maxC: Double
    public let avgC: Double
    public let count: Int

    public var id: String { group }
}

public struct Status: Decodable, Equatable, Sendable {
    public let mode: Mode
    public let profile: Profile
    public let decision: Decision
    public let reason: Reason
    public let hotspotKey: String?
    public let hotspotC: Double?
    public let controlC: Double?
    public let dutyPct: Double
    public let fans: [FanStatus]
    public let groups: [GroupSummary]
    public let controlSensorCount: Int
    public let tickUs: UInt64
    public let ticks: UInt64
    public let smcWrites: UInt64
    public let lastError: String?
    public let externalOverride: Bool
    public let conflicts: [String]
    public let uptimeS: Double
    public let model: String
    public let writesEnabled: Bool
    public let chip: String
    public let support: SupportLevel
    public let supportNote: String
    public let controlDisabled: String?

    private enum CodingKeys: String, CodingKey {
        case mode, profile, decision, reason, hotspotKey, hotspotC, controlC, dutyPct, fans, groups
        case controlSensorCount, tickUs, ticks, smcWrites, lastError, externalOverride, conflicts, uptimeS
        case model, writesEnabled, chip, support, supportNote, controlDisabled
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        mode = try c.decode(Mode.self, forKey: .mode)
        profile = try c.decode(Profile.self, forKey: .profile)
        decision = try c.decode(Decision.self, forKey: .decision)
        reason = try c.decode(Reason.self, forKey: .reason)
        hotspotKey = try c.decodeIfPresent(String.self, forKey: .hotspotKey)
        hotspotC = try c.decodeIfPresent(Double.self, forKey: .hotspotC)
        controlC = try c.decodeIfPresent(Double.self, forKey: .controlC)
        dutyPct = try c.decode(Double.self, forKey: .dutyPct)
        fans = try c.decode([FanStatus].self, forKey: .fans)
        groups = try c.decode([GroupSummary].self, forKey: .groups)
        controlSensorCount = try c.decode(Int.self, forKey: .controlSensorCount)
        tickUs = try c.decode(UInt64.self, forKey: .tickUs)
        ticks = try c.decode(UInt64.self, forKey: .ticks)
        smcWrites = try c.decode(UInt64.self, forKey: .smcWrites)
        lastError = try c.decodeIfPresent(String.self, forKey: .lastError)
        externalOverride = try c.decode(Bool.self, forKey: .externalOverride)
        conflicts = try c.decodeIfPresent([String].self, forKey: .conflicts) ?? []
        uptimeS = try c.decode(Double.self, forKey: .uptimeS)
        model = try c.decodeIfPresent(String.self, forKey: .model) ?? ""
        writesEnabled = try c.decodeIfPresent(Bool.self, forKey: .writesEnabled) ?? true
        chip = try c.decodeIfPresent(String.self, forKey: .chip) ?? ""
        support = try c.decodeIfPresent(SupportLevel.self, forKey: .support) ?? .monitorOnly
        supportNote = try c.decodeIfPresent(String.self, forKey: .supportNote) ?? ""
        controlDisabled = try c.decodeIfPresent(String.self, forKey: .controlDisabled)
    }
}

/// Requests understood by the daemon.
public enum Request: Encodable, Sendable {
    case ping
    case status
    case setMode(Mode)
    case setProfile(Profile)

    private enum CodingKeys: String, CodingKey { case cmd, mode, profile }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .ping: try c.encode("ping", forKey: .cmd)
        case .status: try c.encode("status", forKey: .cmd)
        case let .setMode(m):
            try c.encode("set_mode", forKey: .cmd)
            try c.encode(m, forKey: .mode)
        case let .setProfile(p):
            try c.encode("set_profile", forKey: .cmd)
            try c.encode(p, forKey: .profile)
        }
    }
}

/// Responses we care about. Config payloads are acknowledged but not decoded.
public enum Response: Decodable, Sendable {
    case pong(version: String, protocolVersion: Int)
    case status(Status)
    case config
    case error(String)
    case other(String)

    private enum CodingKeys: String, CodingKey { case type, version, `protocol`, status, message }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let type = try c.decode(String.self, forKey: .type)
        switch type {
        case "pong":
            self = .pong(
                version: try c.decode(String.self, forKey: .version),
                protocolVersion: try c.decode(Int.self, forKey: .protocol))
        case "status": self = .status(try c.decode(Status.self, forKey: .status))
        case "config": self = .config
        case "error": self = .error(try c.decode(String.self, forKey: .message))
        default: self = .other(type)
        }
    }
}

extension JSONDecoder {
    public static var daemon: JSONDecoder {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }
}
