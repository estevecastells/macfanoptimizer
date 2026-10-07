// Checks GitHub for new releases and installs them in place: download, verify
// signature and checksum (DimKit/Updater.swift), swap the app bundle, relaunch.
// The relaunched app then offers to update the bundled fan service (AppModel),
// which needs an administrator password.

import AppKit
import FanOptimizerKit
import Foundation
import Observation

@MainActor
@Observable
public final class Updates {
    public enum State: Equatable {
        case idle
        case checking
        case upToDate
        case available(version: String, notes: URL)
        case installing(String)
        case failed(String)
    }

    public private(set) var state: State = .idle
    private var pending: Release?
    private var loop: Task<Void, Never>?

    private static let autoInstallKey = "autoInstallUpdates"
    /// Set before relaunching after an update the person started; read once by the new process.
    static let promptServiceUpdateKey = "promptServiceUpdate"

    var autoInstall: Bool {
        get { UserDefaults.standard.object(forKey: Self.autoInstallKey) as? Bool ?? true }
        set { UserDefaults.standard.set(newValue, forKey: Self.autoInstallKey) }
    }

    static let checkEvery: Duration = .seconds(6 * 3600)

    public static var currentVersion: Version {
        Version(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "") ?? Version("0")!
    }

    public init() {}

    /// Check shortly after launch, then periodically.
    func start() {
        loop?.cancel()
        loop = Task { [weak self] in
            try? await Task.sleep(for: .seconds(15))
            while !Task.isCancelled {
                await self?.check(userInitiated: false)
                try? await Task.sleep(for: Updates.checkEvery)
            }
        }
    }

    func check(userInitiated: Bool) async {
        if case .installing = state { return }
        state = .checking
        do {
            if let release = try await UpdateCheck.newer(than: Self.currentVersion) {
                pending = release
                state = .available(version: release.version?.description ?? release.tagName, notes: release.htmlUrl)
                if autoInstall && !userInitiated { await install(userInitiated: false) }
            } else {
                pending = nil
                state = .upToDate
            }
        } catch {
            // Background checks fail quietly (offline, rate limit); only show errors the user asked for.
            state = userInitiated ? .failed("Couldn't check for updates: \(error.localizedDescription)") : .idle
        }
    }

    /// `userInitiated`: the person clicked Update, so the relaunched app may ask
    /// for the password to update the fan service straight away.
    func install(userInitiated: Bool = true) async {
        guard let release = pending else { return }
        do {
            try await Self.install(release) { step in self.state = .installing(step) }
            UserDefaults.standard.set(userInitiated, forKey: Self.promptServiceUpdateKey)
            Self.relaunch()
        } catch {
            state = .failed(error.localizedDescription)
        }
    }

    private static func install(_ release: Release, progress: @escaping @MainActor (String) -> Void) async throws {
        let bundle = Bundle.main.bundleURL
        guard bundle.pathExtension == "app" else { throw UpdateError.invalidBundle("not running from an app bundle") }
        let fm = FileManager.default
        // On the same volume as the app, so the final swap is a rename.
        let work = try fm.url(for: .itemReplacementDirectory, in: .userDomainMask, appropriateFor: bundle, create: true)
        defer { try? fm.removeItem(at: work) }

        progress("Downloading…")
        func fetch(_ name: String) async throws -> URL {
            guard let url = release.asset(name) else { throw UpdateError.missingAsset(name) }
            let (tmp, response) = try await URLSession.shared.download(from: url)
            if let http = response as? HTTPURLResponse, http.statusCode != 200 {
                throw UpdateError.missingAsset("\(name) (HTTP \(http.statusCode))")
            }
            let dest = work.appendingPathComponent(name)
            try fm.moveItem(at: tmp, to: dest)
            return dest
        }
        let sums = try await fetch("SHA256SUMS")
        let sig = try await fetch("SHA256SUMS.sig")
        let zip = try await fetch(UpdateConfig.appAsset)

        progress("Verifying…")
        let verifier = try ReleaseVerifier(publicKeyBase64: UpdateConfig.publicKey)
        let entries = try verifier.verifiedSums(sums: Data(contentsOf: sums), signature: Data(contentsOf: sig))
        try ReleaseVerifier.check(zip, named: UpdateConfig.appAsset, in: entries)

        progress("Installing…")
        let unpacked = work.appendingPathComponent("unpacked")
        try run("/usr/bin/ditto", ["-x", "-k", zip.path, unpacked.path])
        guard let newApp = try fm.contentsOfDirectory(at: unpacked, includingPropertiesForKeys: nil)
            .first(where: { $0.pathExtension == "app" })
        else { throw UpdateError.invalidBundle("no app in the download") }
        guard let info = Bundle(url: newApp)?.infoDictionary,
            info["CFBundleIdentifier"] as? String == Bundle.main.bundleIdentifier
        else { throw UpdateError.invalidBundle("unexpected bundle identifier") }
        guard let v = (info["CFBundleShortVersionString"] as? String).flatMap(Version.init), v == release.version
        else { throw UpdateError.invalidBundle("version doesn't match the release") }
        try run("/usr/bin/codesign", ["--verify", "--deep", newApp.path])
        try? run("/usr/bin/xattr", ["-dr", "com.apple.quarantine", newApp.path])

        _ = try fm.replaceItemAt(bundle, withItemAt: newApp)
    }

    private static func run(_ tool: String, _ args: [String]) throws {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: tool)
        p.arguments = args
        p.standardOutput = FileHandle.nullDevice
        p.standardError = FileHandle.nullDevice
        try p.run()
        p.waitUntilExit()
        if p.terminationStatus != 0 {
            throw UpdateError.invalidBundle("\((tool as NSString).lastPathComponent) failed")
        }
    }

    /// Start the new copy once this process has exited, then quit.
    private static func relaunch() {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/sh")
        p.arguments = ["-c", "sleep 1; /usr/bin/open \"$0\"", Bundle.main.bundlePath]
        try? p.run()
        NSApplication.shared.terminate(nil)
    }
}
