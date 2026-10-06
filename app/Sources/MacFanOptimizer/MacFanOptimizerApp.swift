import FanOptimizerKit
import SwiftUI

@main
@MainActor
struct MacFanOptimizerApp: App {
    @State private var model = AppModel()

    var body: some Scene {
        MenuBarExtra {
            MenuContent(model: model)
        } label: {
            MenuBarLabel(status: model.status, connection: model.connection)
        }
        .menuBarExtraStyle(.window)
    }
}

struct MenuBarLabel: View {
    let status: Status?
    let connection: Connection

    var body: some View {
        HStack(spacing: 3) {
            Image(systemName: icon)
            if connection == .connected {
                Text(Format.menuBar(status)).monospacedDigit()
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
