import AppKit
import FanOptimizerKit
import Foundation
import Observation
import ServiceManagement

public enum Connection: Equatable {
    case connecting
    case connected
    /// Daemon not installed or not running.
    case missing
    case failed(String)
}

@MainActor
@Observable
public final class AppModel {
    public private(set) var status: Status?
    public private(set) var connection: Connection = .connecting
    private(set) var actionError: String?
    private(set) var installing = false
    /// Mirrors the login item state; set through `setOpenAtLogin`.
    private(set) var openAtLogin = false
    private(set) var loginItemNeedsApproval = false
    /// The user's Temperature setting, re-read on every poll so a change in
    /// System Settings shows up within one interval.
    public private(set) var temperatureUnit = TemperatureUnit.preferred()
    /// The mode the on/off switch restores. Persisted so it survives relaunches.
    private(set) var lastActiveMode: Mode = AppModel.loadLastActiveMode()

    private let client = DaemonClient()
    private var pollTask: Task<Void, Never>?

    /// Polling period. A status call is a single local socket round-trip.
    static let pollInterval: Duration = .seconds(2)

    public init() {
        configureLoginItemOnFirstLaunch()
        start()
    }

    /// A static model for previews and README screenshots: no daemon, no polling.
    /// Celsius by default so screenshots don't depend on the machine's settings.
    public init(previewStatus: Status, temperatureUnit: TemperatureUnit = .celsius) {
        status = previewStatus
        self.temperatureUnit = temperatureUnit
        connection = .connected
        openAtLogin = true
    }

    // MARK: Open at login

    private static let loginItemConfiguredKey = "loginItemConfigured"

    /// Open at login by default the first time the app runs; after that the
    /// user's choice (from the menu or System Settings) wins.
    private func configureLoginItemOnFirstLaunch() {
        let defaults = UserDefaults.standard
        if !defaults.bool(forKey: Self.loginItemConfiguredKey) {
            defaults.set(true, forKey: Self.loginItemConfiguredKey)
            setOpenAtLogin(true)
        }
        refreshLoginItem()
    }

    func refreshLoginItem() {
        let state = SMAppService.mainApp.status
        openAtLogin = state == .enabled || state == .requiresApproval
        loginItemNeedsApproval = state == .requiresApproval
    }

    func setOpenAtLogin(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            actionError = nil
        } catch {
            actionError = "Couldn't change the login item: \(error.localizedDescription)"
        }
        refreshLoginItem()
    }

    func openLoginItemSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }

    public func start() {
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refresh()
                try? await Task.sleep(for: AppModel.pollInterval)
            }
        }
    }

    func refresh() async {
        let unit = TemperatureUnit.preferred()
        if unit != temperatureUnit { temperatureUnit = unit }
        do {
            let status = try await client.status()
            self.status = status
            connection = .connected
            // Also catches changes made with fanctl.
            rememberActiveMode(status.mode)
        } catch DaemonError.notRunning {
            status = nil
            connection = .missing
        } catch {
            connection = .failed(error.localizedDescription)
        }
    }

    func setMode(_ mode: Mode) {
        rememberActiveMode(mode)
        perform(.setMode(mode))
    }

    // MARK: On/off switch

    private static let lastActiveModeKey = "lastActiveMode"

    /// Off hands the fans to macOS (`system` mode); on restores the last other mode.
    func setControlEnabled(_ enabled: Bool) {
        setMode(enabled ? lastActiveMode : .system)
    }

    private func rememberActiveMode(_ mode: Mode) {
        guard mode != .system, mode != lastActiveMode else { return }
        lastActiveMode = mode
        if let data = try? JSONEncoder().encode(mode) {
            UserDefaults.standard.set(data, forKey: Self.lastActiveModeKey)
        }
    }

    private static func loadLastActiveMode() -> Mode {
        guard let data = UserDefaults.standard.data(forKey: lastActiveModeKey),
            let mode = try? JSONDecoder().decode(Mode.self, from: data), mode != .system
        else { return .smart }
        return mode
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
