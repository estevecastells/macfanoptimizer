// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "MacFanOptimizer",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "MacFanOptimizer", targets: ["MacFanOptimizer"]),
    ],
    targets: [
        // Protocol models and the daemon socket client. No UI; fully testable.
        .target(name: "FanOptimizerKit"),
        // The menu bar app.
        .executableTarget(name: "MacFanOptimizer", dependencies: ["FanOptimizerKit"]),
        // Self-checking test runner. XCTest isn't available with Command Line
        // Tools alone, so `swift run kit-checks` works on any setup.
        .executableTarget(name: "KitChecks", dependencies: ["FanOptimizerKit"], path: "Sources/KitChecks"),
    ]
)
