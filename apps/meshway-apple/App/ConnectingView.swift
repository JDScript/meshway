import SwiftUI

enum ConnectionStep: Int, Equatable {
    case authenticate, synchronize, apply, ready
    var title: String {
        switch self {
        case .authenticate: "正在认证"
        case .synchronize: "正在同步网络"
        case .apply: "正在应用"
        case .ready: "已连接"
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
        }.accessibilityLabel(step?.title ?? "正在等待 Node 状态")
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

struct SkeletonRows: View {
    var count = 3
    var addresses = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 24, paused: reduceMotion)) { context in
            VStack(spacing: 1) {
                ForEach(0..<count, id: \.self) { index in
                    HStack(spacing: 10) {
                        Circle().fill(.quaternary).frame(width: 7, height: 7)
                        Capsule().fill(.quaternary).frame(width: CGFloat(80 + index % 3 * 18), height: 10)
                        Spacer(minLength: 4)
                        if addresses { Capsule().fill(.quaternary).frame(width: 90, height: 10) }
                        Capsule().fill(.quaternary).frame(width: 44, height: 10)
                    }.padding(.horizontal, 10).frame(height: 29)
                }
            }.overlay {
                if !reduceMotion {
                    GeometryReader { geometry in
                        let phase = context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1.6) / 1.6
                        LinearGradient(colors: [.clear, .primary.opacity(0.08), .clear], startPoint: .leading, endPoint: .trailing)
                            .frame(width: 100).offset(x: (geometry.size.width + 100) * phase - 100)
                    }.clipped()
                }
            }
        }.accessibilityElement(children: .ignore).accessibilityLabel("正在载入网络配置")
    }
}

// A disabled preview keeps the real desiredUp color, rather than appearing off.
// Button/Toggle accessibility and disabled semantics still come from SwiftUI.
struct ConnectionSwitchStyle: ToggleStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    func makeBody(configuration: Configuration) -> some View {
        Button { configuration.isOn.toggle() } label: {
            Capsule().fill(configuration.isOn ? Color.teal : Color.secondary.opacity(0.25))
                .frame(width: 34, height: 20)
                .overlay(alignment: configuration.isOn ? .trailing : .leading) {
                    Circle().fill(.white).frame(width: 16, height: 16).shadow(color: .black.opacity(0.15), radius: 1, y: 1).padding(2)
                }
        }.buttonStyle(.plain).accessibilityLabel("连接 Meshway")
            .accessibilityValue(configuration.isOn ? "开" : "关")
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: configuration.isOn)
    }
}
