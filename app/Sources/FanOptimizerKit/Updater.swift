// Self-update from GitHub Releases, without a third-party framework.
//
// Trust: each release publishes SHA256SUMS and SHA256SUMS.sig, an Ed25519
// signature made in CI with a key that only exists as a repository secret
// (scripts/sign-update.swift). The app embeds the public key and installs
// nothing unless the signature and the checksum both verify, so a tampered
// download can't install itself even if the release assets are replaced.

import CryptoKit
import Foundation

public enum UpdateConfig {
    public static let repo = "estevecastells/macfanoptimizer"
    public static let appAsset = "MacFanOptimizer-macos-arm64.zip"
    /// Base64 raw Ed25519 public keys a release may be signed with. The first matches the
    /// `UPDATE_SIGNING_KEY` repository secret; the second is the key it replaced, kept so the
    /// release that introduced the new key could still be signed with the old one.
    public static let publicKeys = [
        "t+w4uDzt9L8nl3UrTEq5EApiWXlTm7ua66SS1uBdbxs=",
        "V2nFnwi7j+AqQ78N//Ov3m6M4YvIIqG04cAY1i1iWqI=",
    ]
    /// `MACFANOPTIMIZER_UPDATE_FEED` points at a different release JSON (a file:// URL works), for testing.
    public static var feedURL: URL {
        if let s = ProcessInfo.processInfo.environment["MACFANOPTIMIZER_UPDATE_FEED"], let u = URL(string: s) { return u }
        return URL(string: "https://api.github.com/repos/\(repo)/releases/latest")!
    }
}

/// Dotted numeric version, "v0.2.10" or "0.2.10".
public struct Version: Comparable, CustomStringConvertible, Sendable {
    public let parts: [Int]

    public init?(_ s: String) {
        let trimmed = s.hasPrefix("v") ? String(s.dropFirst()) : s
        let parts = trimmed.split(separator: ".").map { Int($0) }
        guard !parts.isEmpty, parts.allSatisfy({ $0 != nil }) else { return nil }
        self.parts = parts.compactMap { $0 }
    }

    public static func < (a: Version, b: Version) -> Bool {
        for i in 0..<max(a.parts.count, b.parts.count) {
            let (x, y) = (i < a.parts.count ? a.parts[i] : 0, i < b.parts.count ? b.parts[i] : 0)
            if x != y { return x < y }
        }
        return false
    }

    public static func == (a: Version, b: Version) -> Bool { !(a < b) && !(b < a) }

    public var description: String { parts.map(String.init).joined(separator: ".") }
}

/// The parts of GitHub's release JSON we use.
public struct Release: Decodable, Sendable {
    public struct Asset: Decodable, Sendable {
        public let name: String
        public let browserDownloadUrl: URL
    }

    public let tagName: String
    public let htmlUrl: URL
    public let draft: Bool
    public let prerelease: Bool
    public let assets: [Asset]

    public var version: Version? { Version(tagName) }

    public func asset(_ name: String) -> URL? { assets.first { $0.name == name }?.browserDownloadUrl }

    public static func decode(_ data: Data) throws -> Release {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return try d.decode(Release.self, from: data)
    }
}

public enum UpdateError: Error, LocalizedError, Equatable {
    case badSignature
    case checksumMismatch(String)
    case missingAsset(String)
    case invalidBundle(String)

    public var errorDescription: String? {
        switch self {
        case .badSignature: "The update's signature didn't verify, so it wasn't installed."
        case let .checksumMismatch(name): "\(name) doesn't match its published checksum, so it wasn't installed."
        case let .missingAsset(name): "The release is missing \(name)."
        case let .invalidBundle(why): "The downloaded app looks wrong (\(why)), so it wasn't installed."
        }
    }
}

public struct ReleaseVerifier: Sendable {
    let keys: [Curve25519.Signing.PublicKey]

    public init(publicKeyBase64: String) throws {
        try self.init(publicKeysBase64: [publicKeyBase64])
    }

    /// A signature from any of these keys is accepted.
    public init(publicKeysBase64: [String]) throws {
        keys = try publicKeysBase64.map { b64 in
            guard let raw = Data(base64Encoded: b64) else { throw UpdateError.badSignature }
            return try Curve25519.Signing.PublicKey(rawRepresentation: raw)
        }
        if keys.isEmpty { throw UpdateError.badSignature }
    }

    /// Check `SHA256SUMS` against its base64 signature file, then return its entries (file name → hex digest).
    public func verifiedSums(sums: Data, signature: Data) throws -> [String: String] {
        let text = String(decoding: signature, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        guard let sig = Data(base64Encoded: text), keys.contains(where: { $0.isValidSignature(sig, for: sums) }) else {
            throw UpdateError.badSignature
        }
        return Self.parseSums(String(decoding: sums, as: UTF8.self))
    }

    /// `shasum -a 256` output: "<hex>  <name>" per line.
    public static func parseSums(_ text: String) -> [String: String] {
        var out: [String: String] = [:]
        for line in text.split(separator: "\n") {
            let fields = line.split(separator: " ", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
            if fields.count == 2 { out[fields[1].trimmingCharacters(in: CharacterSet(charactersIn: "*"))] = fields[0].lowercased() }
        }
        return out
    }

    public static func sha256Hex(of file: URL) throws -> String {
        let data = try Data(contentsOf: file, options: .mappedIfSafe)
        return SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    public static func check(_ file: URL, named name: String, in sums: [String: String]) throws {
        guard let expected = sums[name] else { throw UpdateError.missingAsset(name) }
        guard try sha256Hex(of: file) == expected else { throw UpdateError.checksumMismatch(name) }
    }
}

public enum UpdateCheck {
    /// The latest published release, or nil when it isn't newer than `current`.
    public static func newer(than current: Version, feed: URL = UpdateConfig.feedURL) async throws -> Release? {
        var req = URLRequest(url: feed, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 20)
        req.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        let (data, _) = try await URLSession.shared.data(for: req)
        let release = try Release.decode(data)
        guard !release.draft, !release.prerelease, let v = release.version, current < v else { return nil }
        return release
    }
}
