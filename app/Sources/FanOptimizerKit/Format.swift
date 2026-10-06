import Foundation

public enum Format {
    public static func celsius(_ v: Double?) -> String {
        guard let v, v.isFinite else { return "–" }
        return "\(Int(v.rounded()))°"
    }

    public static func rpm(_ v: Double?) -> String {
        guard let v, v.isFinite else { return "–" }
        if v < 1 { return "off" }
        return "\(Int((v / 10).rounded() * 10))"
    }

    /// Compact menu bar text, e.g. "64° 2730".
    public static func menuBar(_ s: Status?) -> String {
        guard let s else { return "–" }
        let fan = s.fans.compactMap { $0.reading?.actualRpm }.max()
        return "\(celsius(s.controlC)) \(rpm(fan))"
    }
}
