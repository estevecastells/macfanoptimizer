import FanOptimizerKit
import SwiftUI

public struct MenuContent: View {
    let model: AppModel

    public init(model: AppModel) {
        self.model = model
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            switch model.connection {
            case .connected:
                if let status = model.status {
                    StatusView(model: model, status: status)
                }
            case .connecting:
                ProgressView("Connecting to fan daemon…").frame(maxWidth: .infinity)
            case .missing:
                SetupView(model: model)
            case let .failed(message):
                Label(message, systemImage: "exclamationmark.triangle").foregroundStyle(.red)
            }

            if let error = model.actionError {
                Text(error).font(.caption).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
            }

            Divider()
            HStack {
                Toggle("Open at login", isOn: Binding(get: { model.openAtLogin }, set: { model.setOpenAtLogin($0) }))
                    .toggleStyle(.checkbox)
                    .font(.callout)
                if model.loginItemNeedsApproval {
                    Button("Approve…") { model.openLoginItemSettings() }
                        .buttonStyle(.link)
                        .font(.caption)
                        .help("macOS needs you to allow MacFanOptimizer in Login Items")
                }
            }
            HStack {
                Button("Log") { model.openLog() }
                Spacer()
                Link("GitHub", destination: URL(string: "https://github.com/estevecastells/macfanoptimizer")!)
                Spacer()
                Button("Quit") { NSApplication.shared.terminate(nil) }
            }
            .buttonStyle(.borderless)
            .font(.callout)
        }
        .padding(14)
        .frame(width: 320)
    }
}

private struct StatusView: View {
    let model: AppModel
    let status: Status

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            header
            warnings
            ForEach(status.fans) { FanRow(fan: $0) }
            Divider()
            ModePicker(model: model, status: status)
            if !status.groups.isEmpty {
                Divider()
                sensors
            }
        }
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(Format.celsius(status.controlC))
                .font(.system(size: 34, weight: .semibold, design: .rounded))
                .monospacedDigit()
                .foregroundStyle(temperatureColor(status.controlC))
            VStack(alignment: .leading, spacing: 2) {
                Text(headline).font(.callout)
                Text(subline).font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    /// What the controller is doing, in plain words.
    private var headline: String {
        switch status.reason {
        case .normal:
            switch status.mode {
            case .smart: return "Smart · fans at \(fanSpeedPercent)% speed"
            case let .fixed(rpm): return "Fixed at \(Format.rpm(rpm)) rpm"
            case .max: return "Max · full speed"
            case .system: return "macOS controls the fans"
            }
        default:
            return status.reason.explanation
        }
    }

    /// Fastest fan as a share of its maximum speed, matching the speed bars.
    private var fanSpeedPercent: Int {
        Int(((status.fans.map(\.fraction).max() ?? 0) * 100).rounded())
    }

    /// Chip temperature context. The big number is smoothed; mention the
    /// instantaneous peak only when it's higher, so the two never look contradictory.
    private var subline: String {
        if let peak = status.hotspotC, let control = status.controlC, peak >= control + 1 {
            return "Chip temperature · peaking at \(Format.celsius(peak))"
        }
        return "Chip temperature"
    }

    @ViewBuilder private var warnings: some View {
        if !status.conflicts.isEmpty {
            Label("Quit \(status.conflicts.joined(separator: ", ")) — it fights over the fans.", systemImage: "exclamationmark.triangle.fill")
                .font(.caption).foregroundStyle(.orange)
        }
        if let why = status.controlDisabled {
            VStack(alignment: .leading, spacing: 4) {
                Label(why, systemImage: "xmark.octagon").font(.caption).foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
                reportLink("Report this Mac so it can be supported")
            }
        } else if status.support == .monitorOnly || !status.writesEnabled {
            VStack(alignment: .leading, spacing: 4) {
                Label(status.supportNote.isEmpty ? "Read-only on this Mac." : status.supportNote, systemImage: "lock")
                    .font(.caption).foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
                if status.support == .monitorOnly { reportLink("Report this Mac") }
            }
        } else if status.support == .compatible {
            VStack(alignment: .leading, spacing: 4) {
                Label(compatibleNote, systemImage: allFansVerified ? "checkmark.seal" : "hourglass")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                if allFansVerified { reportLink("Help validate \(status.model): send a report") }
            }
        }
        if let err = status.lastError {
            Label(err, systemImage: "xmark.octagon").font(.caption).foregroundStyle(.red)
        }
    }

    private var allFansVerified: Bool {
        !status.fans.isEmpty && status.fans.allSatisfy { $0.verified == true }
    }

    private var compatibleNote: String {
        allFansVerified
            ? "\(status.model) isn't validated yet, but the fans responded to control."
            : "\(status.model) isn't validated yet — checking that the fans respond…"
    }

    private func reportLink(_ title: String) -> some View {
        Link(title, destination: URL(string: "https://github.com/estevecastells/macfanoptimizer/issues/new?template=model_support.yml")!)
            .font(.caption)
    }

    private var sensors: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(status.groups.prefix(5)) { g in
                HStack {
                    Text(g.group).font(.caption)
                    Spacer()
                    Text("\(Format.celsius(g.avgC)) avg · \(Format.celsius(g.maxC)) max")
                        .font(.caption).monospacedDigit().foregroundStyle(.secondary)
                }
            }
        }
    }
}

private struct FanRow: View {
    let fan: FanStatus

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Image(systemName: "fan")
                Text("Fan \(fan.info.index + 1)")
                Spacer()
                Text("\(Format.rpm(fan.reading?.actualRpm)) rpm").monospacedDigit()
                if fan.reading?.forced == false {
                    Text("macOS").font(.caption2).foregroundStyle(.secondary)
                }
            }
            .font(.callout)
            SpeedBar(fraction: fan.fraction)
        }
    }
}

@MainActor
private struct ModePicker: View {
    let model: AppModel
    let status: Status
    @State private var fixedRpm: Double = 4000

    private var minRpm: Double { status.fans.map(\.info.minRpm).min() ?? 1000 }
    private var maxRpm: Double { status.fans.map(\.info.maxRpm).max() ?? 7000 }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Picker("Mode", selection: modeBinding) {
                Text("macOS").tag("system")
                Text("Smart").tag("smart")
                Text("Fixed").tag("fixed")
                Text("Max").tag("max")
            }
            .pickerStyle(.segmented)
            .labelsHidden()

            switch status.mode {
            case .smart:
                Picker("Profile", selection: profileBinding) {
                    ForEach([Profile.quiet, .balanced, .performance], id: \.self) { Text($0.title).tag($0) }
                    if status.profile == .custom { Text("Custom").tag(Profile.custom) }
                }
                .pickerStyle(.segmented)
                Text(profileHint).font(.caption).foregroundStyle(.secondary)
            case let .fixed(rpm):
                VStack(alignment: .leading) {
                    Slider(value: $fixedRpm, in: minRpm...maxRpm, step: 100) { editing in
                        if !editing { model.setMode(.fixed(rpm: fixedRpm)) }
                    }
                    Text("\(Int(fixedRpm)) rpm — raised automatically if the chip gets too hot")
                        .font(.caption).foregroundStyle(.secondary)
                }
                .onAppear { fixedRpm = rpm }
            case .system:
                Text("macOS controls the fans. On Apple Silicon this keeps them off until the chip is very hot.")
                    .font(.caption).foregroundStyle(.secondary)
            case .max:
                Text("All fans at full speed.").font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private var profileHint: String {
        switch status.profile {
        case .quiet: "Fans start around 66 °C, full speed at 92 °C."
        case .balanced: "Fans start around 58 °C, full speed at 83 °C."
        case .performance: "Fans start around 50 °C, full speed at 76 °C."
        case .custom: "Custom curve from the config file."
        }
    }

    private var modeBinding: Binding<String> {
        Binding(
            get: { status.mode.kind },
            set: { kind in
                switch kind {
                case "system": model.setMode(.system)
                case "smart": model.setMode(.smart)
                case "max": model.setMode(.max)
                default:
                    let current = status.fans.compactMap { $0.reading?.actualRpm }.max() ?? 4000
                    fixedRpm = min(max(current, minRpm), maxRpm)
                    model.setMode(.fixed(rpm: fixedRpm))
                }
            })
    }

    private var profileBinding: Binding<Profile> {
        Binding(get: { status.profile }, set: { model.setProfile($0) })
    }
}

private struct SetupView: View {
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("Fan daemon not running", systemImage: "fan.slash").font(.headline)
            Text("MacFanOptimizer controls the fans through a small background service that runs as root. Installing it asks for your administrator password once.")
                .font(.callout).fixedSize(horizontal: false, vertical: true)
            Button {
                model.installDaemon()
            } label: {
                if model.installing { ProgressView().controlSize(.small) } else { Text("Install Fan Service…") }
            }
            .disabled(model.installing)
            .keyboardShortcut(.defaultAction)
            Text("Or from a checkout: make install").font(.caption).foregroundStyle(.secondary)
        }
    }
}

/// Fan speed bar. Drawn in SwiftUI rather than with ProgressView, whose AppKit
/// control turns grey whenever the menu window isn't key.
private struct SpeedBar: View {
    let fraction: Double

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                Capsule().fill(.quaternary)
                Capsule()
                    .fill(fraction > 0.9 ? Color.orange.gradient : Color.blue.gradient)
                    .frame(width: max(geo.size.width * fraction, fraction > 0 ? 6 : 0))
            }
        }
        .frame(height: 6)
        .animation(.easeOut(duration: 0.4), value: fraction)
        .accessibilityLabel("Fan speed")
        .accessibilityValue("\(Int((fraction * 100).rounded())) percent of maximum")
    }
}

private func temperatureColor(_ c: Double?) -> Color {
    guard let c else { return .secondary }
    switch c {
    case ..<65: return .primary
    case ..<80: return .orange
    default: return .red
    }
}
