import FanOptimizerUI
import ServiceManagement
import SwiftUI

@main
@MainActor
struct MacFanOptimizerApp: App {
    @State private var model: AppModel

    init() {
        // Diagnostics: `MacFanOptimizer.app/Contents/MacOS/MacFanOptimizer --login-item-status`
        if CommandLine.arguments.contains("--login-item-status") {
            let names: [SMAppService.Status: String] = [
                .enabled: "enabled", .notRegistered: "not registered",
                .requiresApproval: "requires approval in System Settings › Login Items", .notFound: "not found",
            ]
            print("open at login: \(names[SMAppService.mainApp.status] ?? "unknown")")
            exit(0)
        }
        _model = State(initialValue: AppModel())
    }

    var body: some Scene {
        MenuBarExtra {
            MenuContent(model: model)
        } label: {
            MenuBarLabel(reading: model.menuBar)
        }
        .menuBarExtraStyle(.window)
    }
}
