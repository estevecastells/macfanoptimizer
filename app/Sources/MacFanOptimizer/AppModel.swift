import AppKit
import FanOptimizerKit
import Foundation
import Observation

enum Connection: Equatable {
    case connecting
    case connected
    /// Daemon not installed or not running.
    case missing
    case failed(String)
}

@MainActor
@Observable
final class AppModel {
    private(set) var status: Status?
    private(set) var connection: Connection = .connecting
    private(set) var actionError: String?
    private(set) var installing = false

    private let client = DaemonClient()
    private var pollTask: Task<Void, Never>?

    /// Polling period. A status call is a single local socket round-trip.
    static let pollInterval: Duration = .seconds(2)

    init() {
        start()
    }

    func start() {
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refresh()
                try? await Task.sleep(for: AppModel.pollInterval)
            }
        }
    }

    func refresh() async {
        do {
            status = try await client.status()
            connection = .connected
        } catch DaemonError.notRunning {
            status = nil
            connection = .missing
        } catch {
            connection = .failed(error.localizedDescription)
        }
    }

    func setMode(_ mode: Mode) {
        perform(.setMode(mode))
    }

    func setProfile(_ profile: Profile) {
        perform(.setProfile(profile))
    }

    private func perform(_ request: Request) {
        Task {
            do {
                _ = try await client.send(request)
                actionError = nil
            } catch {
                actionError = error.localizedDescription
            }
            await refresh()
        }
    }

    /// Install (or reinstall) the daemon from the app bundle, asking for an
    /// administrator password through the standard macOS prompt.
    func installDaemon() {
        guard let resources = Bundle.main.resourceURL,
            FileManager.default.fileExists(atPath: resources.appendingPathComponent("install.sh").path)
        else {
            actionError = "Installer not found in the app bundle. Run `make install` from the repository instead."
            return
        }
        installing = true
        let script = resources.appendingPathComponent("install.sh").path
        let command = "/bin/bash \(shellQuote(script)) --bin-dir \(shellQuote(resources.path)) --uid \(getuid())"
        Task.detached {
            var error: NSDictionary?
            let source = "do shell script \"\(command.replacingOccurrences(of: "\"", with: "\\\""))\" with administrator privileges"
            NSAppleScript(source: source)?.executeAndReturnError(&error)
            let message = error?[NSAppleScript.errorMessage] as? String
            await MainActor.run {
                self.installing = false
                self.actionError = message
            }
            try? await Task.sleep(for: .seconds(1))
            await self.refresh()
        }
    }

    func openLog() {
        NSWorkspace.shared.open(URL(fileURLWithPath: "/var/log/macfanoptimizer.log"))
    }
}

private func shellQuote(_ s: String) -> String {
    "'" + s.replacingOccurrences(of: "'", with: "'\\''") + "'"
}
