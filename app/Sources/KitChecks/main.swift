// Protocol conformance checks for FanOptimizerKit: `swift run kit-checks`.
// Exits non-zero on failure. Wire fixtures live in /fixtures and are shared
// with the Rust tests, so the two sides can't drift apart silently.

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
}(), "decodes a full status response")

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
check(Format.celsius(61.5) == "62°", "celsius rounding")
check(Format.rpm(0) == "off" && Format.rpm(2729) == "2730", "rpm formatting")

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
