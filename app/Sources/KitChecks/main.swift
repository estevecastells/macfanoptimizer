// Protocol conformance checks for FanOptimizerKit: `swift run kit-checks`.
// Exits non-zero on failure. Wire fixtures live in /fixtures and are shared
// with the Rust tests, so the two sides can't drift apart silently.

import CryptoKit
import FanOptimizerKit
import Foundation

var failures = 0
var passed = 0

func check(_ condition: @autoclosure () throws -> Bool, _ name: String, file: StaticString = #file, line: UInt = #line) {
    do {
        if try condition() {
            passed += 1
        } else {
            failures += 1
            print("FAIL \(name) (\(file):\(line))")
        }
    } catch {
        failures += 1
        print("FAIL \(name): threw \(error)")
    }
}

func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
    try JSONDecoder.daemon.decode(T.self, from: Data(json.utf8))
}

func encode<T: Encodable>(_ v: T) throws -> String {
    let e = JSONEncoder()
    e.outputFormatting = .sortedKeys
    return String(decoding: try e.encode(v), as: UTF8.self)
}

let repoRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
func fixture(_ name: String) -> String {
    try! String(contentsOf: repoRoot.appendingPathComponent("fixtures/\(name)"), encoding: .utf8)
}
let statusJSON = try! String(contentsOf: repoRoot.appendingPathComponent("fixtures/status_response.json"), encoding: .utf8)
let requestFixtures = try! String(contentsOf: repoRoot.appendingPathComponent("fixtures/requests.jsonl"), encoding: .utf8)
    .split(separator: "\n").map(String.init)

// Responses
check(try {
    guard case let .status(s) = try decode(Response.self, statusJSON) else { return false }
    return s.mode == .smart && s.profile == .balanced && s.decision == .duty(pct: 7.48)
        && s.controlC == 61.5 && s.fans.count == 2 && s.fans[0].reading?.forced == true
        && s.fans[1].reading == nil && s.groups.first?.count == 23 && s.conflicts == ["Macs Fan Control"]
        && s.model == "Mac17,9" && s.writesEnabled
        && s.chip == "Apple M5 Pro" && s.support == .validated && s.controlDisabled == nil
        && s.fans[0].verified == true && s.fans[1].verified == nil
}(), "decodes a full status response")

check(try decode(Reason.self, "\"control_disabled\"") == .controlDisabled, "control_disabled reason")
check(try decode(SupportLevel.self, "\"monitor_only\"") == .monitorOnly, "support level")

check(try {
    guard case let .status(s) = try decode(Response.self, statusJSON) else { return false }
    return abs(s.fans[0].fraction - 5989.0 / 7826.0) < 1e-9 && s.fans[1].fraction == 0
}(), "fan fraction")

check(try decode(Reason.self, "\"sensor_failure\"") == .sensorFailure, "snake_case reason values")
check(try decode(Reason.self, "\"something_new\"") == .normal, "unknown reasons degrade gracefully")
check(try decode(Decision.self, #"{"kind":"rpm","rpm":3000,"floor_pct":20}"#) == .rpm(rpm: 3000, floorPct: 20), "rpm decision")
check(try decode(Decision.self, #"{"kind":"system"}"#) == .system, "system decision")
check(try decode(Mode.self, #"{"kind":"fixed","rpm":3500}"#) == .fixed(rpm: 3500), "fixed mode")

check(try {
    guard case let .error(m) = try decode(Response.self, #"{"type":"error","message":"permission denied for uid 501"}"#) else { return false }
    return m.contains("permission denied")
}(), "error response")

check(try {
    guard case let .pong(v, p) = try decode(Response.self, #"{"type":"pong","version":"0.1.0","protocol":1}"#) else { return false }
    return v == "0.1.0" && p == 1
}(), "pong response")

// Requests must match the shared fixtures, which the Rust tests also parse.
let requests: [Request] = [.status, .setMode(.fixed(rpm: 3000)), .setMode(.smart), .setProfile(.quiet)]
check(requestFixtures.count == requests.count, "request fixture count")
for (req, expected) in zip(requests, requestFixtures) {
    check(try encode(req) == expected, "request encodes as \(expected)")
}

// Formatting
check(Format.temperature(61.5, .celsius) == "62°", "celsius rounding")
check(Format.temperature(61.5, .fahrenheit) == "143°", "fahrenheit conversion and rounding")
check(Format.temperature(nil, .fahrenheit) == "–" && Format.temperature(.nan, .celsius) == "–", "missing temperature")
check(Format.degrees(95, .celsius) == "95 °C" && Format.degrees(95, .fahrenheit) == "203 °F", "degrees with unit")
check(try {
    guard case let .status(s) = try decode(Response.self, statusJSON) else { return false }
    return Format.menuBar(s, .celsius) == "62° 5990" && Format.menuBar(s, .fahrenheit) == "143° 5990"
}(), "menu bar text in both units")

// Temperature unit preference: the explicit setting wins over the region's convention.
check(TemperatureUnit.resolve(setting: "Fahrenheit", region: "ES") == .fahrenheit, "Fahrenheit setting")
check(TemperatureUnit.resolve(setting: "Celsius", region: "US") == .celsius, "Celsius setting overrides US region")
check(TemperatureUnit.resolve(setting: nil, region: "US") == .fahrenheit, "US region defaults to Fahrenheit")
check(TemperatureUnit.resolve(setting: nil, region: "ES") == .celsius, "other regions default to Celsius")
check(TemperatureUnit.resolve(setting: nil, region: nil) == .celsius, "no region defaults to Celsius")
check(Format.rpm(0) == "off" && Format.rpm(2729) == "2730", "rpm formatting")

// Updates: versions, release JSON, signatures and checksums.
check(Version("v0.10.0")! > Version("0.9.9")! && Version("1.0")! == Version("1.0.0")! && Version("0.1.0")! < Version("0.1.1")!,
      "version ordering")
check(Version("v1.x") == nil && Version("") == nil, "invalid versions rejected")
check(try {
    let r = try Release.decode(Data(fixture("release_latest.json").utf8))
    return r.version == Version("0.2.0") && !r.draft && r.asset(UpdateConfig.appAsset) != nil && r.asset("nope") == nil
}(), "decodes a GitHub release")
check((try? ReleaseVerifier(publicKeyBase64: UpdateConfig.publicKey)) != nil, "embedded public key is valid")

let testKey = Curve25519.Signing.PrivateKey()
let payload = Data("abc123  MacFanOptimizer-macos-arm64.zip\ndef456  other.tar.gz\n".utf8)
let verifier = try! ReleaseVerifier(publicKeyBase64: testKey.publicKey.rawRepresentation.base64EncodedString())
let goodSig = Data(try! testKey.signature(for: payload).base64EncodedString().utf8)
check(try verifier.verifiedSums(sums: payload, signature: goodSig)["MacFanOptimizer-macos-arm64.zip"] == "abc123",
      "valid signature accepted")
check((try? verifier.verifiedSums(sums: payload + Data("x".utf8), signature: goodSig)) == nil, "tampered sums rejected")
let otherSig = Data(try! Curve25519.Signing.PrivateKey().signature(for: payload).base64EncodedString().utf8)
check((try? verifier.verifiedSums(sums: payload, signature: otherSig)) == nil, "signature from another key rejected")

let tmpFile = FileManager.default.temporaryDirectory.appendingPathComponent("kitchecks-\(getpid()).bin")
try! Data("hello".utf8).write(to: tmpFile)
let helloHex = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
check((try? ReleaseVerifier.check(tmpFile, named: "f", in: ["f": helloHex])) != nil, "checksum match")
check((try? ReleaseVerifier.check(tmpFile, named: "f", in: ["f": String(repeating: "0", count: 64)])) == nil,
      "checksum mismatch rejected")
try? FileManager.default.removeItem(at: tmpFile)

// Client error mapping: connecting to a missing socket means "not running".
let semaphore = DispatchSemaphore(value: 0)
Task {
    do {
        _ = try await DaemonClient(socketPath: "/tmp/definitely-missing-\(getpid()).sock").status()
        check(false, "missing socket should throw")
    } catch {
        check((error as? DaemonError) == .notRunning, "missing socket maps to notRunning")
    }
    semaphore.signal()
}
semaphore.wait()

print("\(passed) passed, \(failures) failed")
exit(failures == 0 ? 0 : 1)
