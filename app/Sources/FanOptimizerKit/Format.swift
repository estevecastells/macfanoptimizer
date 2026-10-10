import Foundation

/// How temperatures are shown. The daemon always speaks Celsius; this is display only.
public enum TemperatureUnit: String, Sendable {
    case celsius, fahrenheit

    /// Regions whose default temperature unit is Fahrenheit (CLDR unit preferences),
    /// used when the user hasn't picked a unit explicitly.
    static let fahrenheitRegions: Set<String> = ["US", "BS", "BZ", "KY", "PR", "PW"]

    /// The unit chosen in System Settings › General › Language & Region › Temperature.
    /// That setting is the global default `AppleTemperatureUnit`. `Locale` and
    /// `MeasurementFormatter` ignore it (checked on macOS 26: they follow the region
    /// only), so read it directly and fall back to the region's convention when unset.
    /// Cheap: `UserDefaults` caches and cfprefsd keeps the global domain current.
    public static func preferred(defaults: UserDefaults = .standard, locale: Locale = .current) -> TemperatureUnit {
        resolve(setting: defaults.string(forKey: "AppleTemperatureUnit"), region: locale.region?.identifier)
    }

    public static func resolve(setting: String?, region: String?) -> TemperatureUnit {
        switch setting {
        case "Fahrenheit": return .fahrenheit
        case "Celsius": return .celsius
        default: return region.map(fahrenheitRegions.contains) == true ? .fahrenheit : .celsius
        }
    }

    public var symbol: String {
        switch self {
        case .celsius: "°C"
        case .fahrenheit: "°F"
        }
    }

    public func convert(_ celsius: Double) -> Double {
        switch self {
        case .celsius: celsius
        case .fahrenheit: celsius * 9 / 5 + 32
        }
    }
}

public enum Format {
    /// Compact reading in whole degrees, e.g. "62°".
    public static func temperature(_ celsius: Double?, _ unit: TemperatureUnit) -> String {
        guard let celsius, celsius.isFinite else { return "–" }
        return "\(Int(unit.convert(celsius).rounded()))°"
    }

    /// With the unit, for prose such as profile hints, e.g. "83 °C" or "181 °F".
    public static func degrees(_ celsius: Double, _ unit: TemperatureUnit) -> String {
        "\(Int(unit.convert(celsius).rounded())) \(unit.symbol)"
    }

    public static func rpm(_ v: Double?) -> String {
        guard let v, v.isFinite else { return "–" }
        if v < 1 { return "off" }
        return "\(Int((v / 10).rounded() * 10))"
    }

    /// Compact menu bar text, e.g. "64° 2730".
    public static func menuBar(_ s: Status?, _ unit: TemperatureUnit) -> String {
        guard let s else { return "–" }
        let fan = s.fans.compactMap { $0.reading?.actualRpm }.max()
        return "\(temperature(s.controlC, unit)) \(rpm(fan))"
    }
}

/// Everything the menu bar shows: an SF Symbol and, when connected, the compact
/// reading. Equatable so the app redraws the status item only when what's on
/// screen changes, not on every poll (a status always differs in its counters).
public struct MenuBarReading: Equatable, Sendable {
    public let icon: String
    public let text: String?

    public static let disconnected = MenuBarReading(icon: "fan.slash", text: nil)

    public init(icon: String, text: String?) {
        self.icon = icon
        self.text = text
    }

    public init(status: Status?, connected: Bool, unit: TemperatureUnit) {
        guard connected else {
            self = .disconnected
            return
        }
        text = Format.menuBar(status, unit)
        switch status?.reason {
        case nil: icon = "fan.slash"
        case .critical?, .protecting?: icon = "flame"
        case .sensorFailure?: icon = "exclamationmark.triangle"
        default: icon = "fan"
        }
    }
}
