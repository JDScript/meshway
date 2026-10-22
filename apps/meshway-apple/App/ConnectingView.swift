import SwiftUI

enum ConnectionStep: Int, Equatable {
    case authenticate, synchronize, apply, ready
    var title: String {
        switch self {
        case .authenticate: String(localized: "Signing in…")
        case .synchronize: String(localized: "Getting network…")
        case .apply: String(localized: "Applying…")
        case .ready: String(localized: "Connected")
        }
    }
    static func resolve(phase: ClientPhase?, plane: ClientPlane?) -> Self? {
        switch phase {
        case .starting: .authenticate
        case .synchronizing: .synchronize
        case .controlReady: .apply
        case .running where plane == .preparing: .apply
        case .running where plane == .ready: .ready
        default: nil
        }
    }
}

struct ConnectionSteps: View {
    let step: ConnectionStep?
    var body: some View {
        HStack(spacing: 4) {
            ForEach(0..<4) { index in
                Capsule().fill(step.map { index <= $0.rawValue } == true ? Color.teal : Color.secondary.opacity(0.18))
                    .frame(width: 16, height: 3)
            }
        }.accessibilityLabel(step?.title ?? String(localized: "Connecting…"))
    }
}

struct ConnectionDot: View {
    let color: Color
    let connecting: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 24, paused: !connecting || reduceMotion)) { context in
            let pulse = connecting && !reduceMotion ? 0.7 + 0.3 * cos(context.date.timeIntervalSinceReferenceDate * .pi * 1.6) : 1
            Circle().fill(color).frame(width: 8, height: 8).opacity(pulse).scaleEffect(0.85 + 0.15 * pulse)
        }
    }
}

/// Soft ease-in-out between `low` and `high`; `offset` staggers neighbours.
/// Shared by the skeleton and loader so both breathe at one tempo.
enum Breath {
    static let period = 1.6
    static func value(at time: TimeInterval, offset: Double = 0, low: Double, high: Double) -> Double {
        let phase = (time / period - offset).truncatingRemainder(dividingBy: 1)
        return low + (high - low) * (0.5 - 0.5 * cos(phase * 2 * .pi))
    }
}

/// The brand mark's three nodes light in turn; the accent node is teal.
/// With Reduce Motion it is static and shows its label as text instead.
struct MeshLoader: View {
    var size: CGFloat = 14
    var label: String = String(localized: "Loading…")
    var labelVisible = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        HStack(spacing: 6) {
            if reduceMotion { mark(time: nil) }
            else { TimelineView(.animation(minimumInterval: 1.0 / 30)) { context in mark(time: context.date.timeIntervalSinceReferenceDate) } }
            if reduceMotion || labelVisible { Text(label).font(.system(size: max(11, size * 0.85))).foregroundStyle(.secondary) }
        }.accessibilityElement(children: .ignore).accessibilityLabel(label)
    }
    static func level(node: Int, time: TimeInterval?) -> Double {
        guard let time else { return 1 }
        return Breath.value(at: time, offset: Double(node) / 3, low: 0.25, high: 1)
    }
    private func mark(time: TimeInterval?) -> some View {
        Canvas { context, canvas in
            let s = canvas.width / 24
            let points = [CGPoint(x: 12 * s, y: 6.5 * s), CGPoint(x: 17.5 * s, y: 15.5 * s), CGPoint(x: 6.5 * s, y: 15.5 * s)]
            var edges = Path()
            edges.move(to: points[0]); edges.addLine(to: points[1]); edges.addLine(to: points[2]); edges.closeSubpath()
            context.stroke(edges, with: .color(.secondary.opacity(0.45)), style: StrokeStyle(lineWidth: max(1, 1.5 * s), lineJoin: .round))
            for (index, point) in points.enumerated() {
                let radius = max(2.4 * s, 1.8)
                let dot = Path(ellipseIn: CGRect(x: point.x - radius, y: point.y - radius, width: radius * 2, height: radius * 2))
                context.fill(dot, with: .color((index == 1 ? Color.teal : Color.primary).opacity(Self.level(node: index, time: time))))
            }
        }.frame(width: size, height: size)
    }
}

/// Each bar breathes on its own with a small row-to-row lag; no sweep.
struct SkeletonRows: View {
    var count = 3
    var addresses = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    static func opacity(row: Int, time: TimeInterval?) -> Double {
        guard let time else { return 0.6 }
        return Breath.value(at: time, offset: Double(row) * 0.08, low: 0.45, high: 0.8)
    }
    var body: some View {
        Group {
            if reduceMotion { rows(time: nil) }
            else { TimelineView(.animation(minimumInterval: 1.0 / 30)) { context in rows(time: context.date.timeIntervalSinceReferenceDate) } }
        }.accessibilityElement(children: .ignore).accessibilityLabel(Text("Loading…"))
    }
    private func rows(time: TimeInterval?) -> some View {
        VStack(spacing: 1) {
            ForEach(0..<count, id: \.self) { index in
                HStack(spacing: 10) {
                    Circle().frame(width: 7, height: 7)
                    Capsule().frame(width: CGFloat(80 + index % 3 * 18), height: 10)
                    Spacer(minLength: 4)
                    if addresses { Capsule().frame(width: 90, height: 10) }
                    Capsule().frame(width: 44, height: 10)
                }.foregroundStyle(Color.primary.opacity(0.16)).opacity(Self.opacity(row: index, time: time))
                    .padding(.horizontal, 10).frame(height: 29)
            }
        }
    }
}

// A disabled preview keeps the real desiredUp color, rather than appearing off.
// Button/Toggle accessibility and disabled semantics still come from SwiftUI.
struct ConnectionSwitchStyle: ToggleStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    func makeBody(configuration: Configuration) -> some View {
        Button { configuration.isOn.toggle() } label: {
            Circle().fill(.white).frame(width: 16, height: 16).shadow(color: .black.opacity(0.15), radius: 1, y: 1).padding(2)
                .frame(width: 34, height: 20, alignment: configuration.isOn ? .trailing : .leading)
        }.buttonStyle(.plain)
            .background(configuration.isOn ? Color.teal : Color.secondary.opacity(0.25), in: Capsule())
            .accessibilityLabel(Text("Connect Meshway"))
            .accessibilityValue(configuration.isOn ? Text("On") : Text("Off"))
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: configuration.isOn)
    }
}
