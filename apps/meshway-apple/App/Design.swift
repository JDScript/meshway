import SwiftUI

@MainActor @Observable
final class AppNavigation {
    static let shared = AppNavigation()
    var page: AppPage? = .peers
}

struct SampleBanner: View {
    var body: some View {
        #if DEBUG
        Text("SAMPLE").font(.system(size: 8, weight: .medium)).tracking(0.5)
            .foregroundStyle(.tertiary).padding(.horizontal, 5).padding(.vertical, 2)
            .background(.quaternary.opacity(0.5), in: Capsule())
            .help("Debug 样例 · 系统操作已禁用")
        #else
        EmptyView()
        #endif
    }
}

struct RelativeTime: View {
    let date: Date
    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { _ in
            Text(date, format: .relative(presentation: .named)).foregroundStyle(.secondary)
                .contentTransition(.numericText()).lineLimit(1)
                .help(date.formatted(date: .complete, time: .complete))
        }
    }
}
struct OptionalTime: View {
    let date: Date?
    var body: some View {
        if let date { RelativeTime(date: date) } else { Text("未报告").foregroundStyle(.secondary) }
    }
}

struct CopyButton: View {
    let value: String
    var title = "复制地址"
    @State private var copied = false
    @State private var resetID = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        Button { copy(value); copied = true; resetID += 1 } label: {
            Image(systemName: copied ? "checkmark" : "doc.on.doc")
                .contentTransition(reduceMotion ? .identity : .symbolEffect(.replace))
                .foregroundStyle(copied ? Color.teal : Color.secondary).frame(width: 18, height: 18)
        }.buttonStyle(.borderless).help(copied ? "已复制" : title).accessibilityLabel(copied ? "已复制" : title)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: copied)
            .task(id: resetID) {
                guard copied else { return }
                do { try await Task.sleep(for: .seconds(1.6)) } catch { return }
                copied = false
            }
    }
}
struct AddressRow: View {
    let title: String
    let value: String?
    var missing = "未分配"
    var body: some View {
        LabeledContent(title) {
            HStack(spacing: 8) {
                Text(value ?? missing).font(value == nil ? .callout : .callout.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                if let value { CopyButton(value: value, title: "复制 \(title)") }
                else { Color.clear.frame(width: 18, height: 18) }
            }
        }
    }
}
@MainActor func unavailableHelp(_ model: AppModel, fallback: String) -> String {
    if model.sampleMode { return "样例预览 — 系统操作已禁用" }
    if model.busy { return "正在等待当前操作完成" }
    return fallback
}
func copy(_ value: String) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(value, forType: .string) }
func human<T>(_ value: T) -> String {
    String(describing: value).replacingOccurrences(of: "([a-z])([A-Z])", with: "$1 $2", options: .regularExpression).capitalized
}
func milliseconds(_ value: UInt64) -> String { String(format: "%.2f ms", Double(value) / 1000) }
