import FanOptimizerKit
import SwiftUI

public struct MenuBarLabel: View {
    let status: Status?
    let connection: Connection
    let unit: TemperatureUnit

    public init(status: Status?, connection: Connection, unit: TemperatureUnit) {
        self.status = status
        self.connection = connection
        self.unit = unit
    }

    public var body: some View {
        HStack(spacing: 3) {
            Image(systemName: icon)
            if connection == .connected {
                Text(Format.menuBar(status, unit)).monospacedDigit()
            }
        }
    }

    private var icon: String {
        guard connection == .connected, let status else { return "fan.slash" }
        switch status.reason {
        case .critical, .protecting: return "flame"
        case .sensorFailure: return "exclamationmark.triangle"
        default: return "fan"
        }
    }
}
