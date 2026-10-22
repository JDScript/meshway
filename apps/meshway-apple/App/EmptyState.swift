import SwiftUI

enum EmptyStateKind: Equatable {
    case devices, search(String), mesh, profiles, serverUnavailable, serverCached(Date)
    var title: String {
        switch self {
        case .devices: "还没有其他设备"
        case .search(let query): "没有匹配“\(query)”的设备"
        case .mesh: "只有 Mesh 方案"
        case .profiles: "尚无出口方案"
        case .serverUnavailable, .serverCached: "暂时连不上 Server"
        }
    }
    var symbol: String {
        switch self {
        case .devices: "desktopcomputer"
        case .search: "magnifyingglass"
        case .mesh, .profiles: "arrow.up.forward"
        case .serverUnavailable, .serverCached: "exclamationmark.icloud"
        }
    }
    var explanation: String {
        switch self {
        case .devices: "在其他设备上安装 Meshway 并加入这个网络后，会出现在这里。"
        case .search: "试试名称或 Mesh IP 的一部分。"
        case .mesh: "管理员可以在 Console 为你分配其他出口方案。"
        case .profiles: "安装网络配置后，可用方案会出现在这里。"
        case .serverUnavailable: "尚不能确认缓存网络可用。等待有效租约与数据面就绪。"
        case .serverCached: "已在用缓存的网络继续工作。"
        }
    }
    static func server(sync: ClientSync?, issue: ClientSyncError?, ready: Bool, applied: Bool, expires: Date?, now: Date) -> Self? {
        guard let sync, sync != .connected, issue == nil || issue == .network || issue == .serverBusy else { return nil }
        if ready, applied, let expires, expires > now { return .serverCached(expires) }
        return .serverUnavailable
    }
    static func devices(query: String, count: Int, server: Self?) -> Self? {
        guard count == 0 else { return nil }
        return query.isEmpty ? (server ?? .devices) : .search(query)
    }
}

@MainActor
func serverEmptyState(_ model: AppModel, now: Date = .now) -> EmptyStateKind? {
    guard model.status?.desiredUp == true else { return nil }
    return EmptyStateKind.server(sync: model.status?.sync, issue: model.status?.syncError,
                                 ready: model.status?.phase == .running && model.status?.dataPlane == .ready,
                                 applied: model.netmapApplied, expires: model.status?.netmap?.expiresAt, now: now)
}

struct QuietEmptyState: View {
    let symbol: String
    let title: String
    let explanation: String
    var compact = false
    var actionTitle: String?
    var action: (() -> Void)?
    var actionDisabled = false
    var actionHelp = ""
    var body: some View {
        VStack(spacing: compact ? 6 : 9) {
            Image(systemName: symbol).font(.system(size: compact ? 18 : 24, weight: .light))
                .foregroundStyle(.teal).frame(width: compact ? 36 : 50, height: compact ? 36 : 50)
                .glassEffect(.regular, in: .rect(cornerRadius: compact ? 12 : 16))
            Text(title).font(.system(size: compact ? 12 : 14, weight: .semibold)).lineLimit(2)
            Text(explanation).font(.system(size: compact ? 11 : 12)).foregroundStyle(.secondary).lineLimit(4)
            if let actionTitle, let action {
                Button(actionTitle, action: action).buttonStyle(.glass).tint(.teal)
                    .disabled(actionDisabled).help(actionHelp)
            }
        }.multilineTextAlignment(.center).frame(maxWidth: compact ? 270 : 320)
            .frame(maxWidth: .infinity).frame(height: compact ? 132 : 196)
    }
}

struct EmptyStateView: View {
    let kind: EmptyStateKind
    var compact = false
    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            QuietEmptyState(symbol: kind.symbol, title: kind.title, explanation: explanation(now: context.date), compact: compact)
        }
    }
    private func explanation(now: Date) -> String {
        guard case .serverCached(let expiry) = kind else { return kind.explanation }
        guard expiry > now else { return "缓存租约已到期，等待重新同步；不能继续使用过期授权。" }
        let remaining = expiry.formatted(.relative(presentation: .numeric))
        return "已在用缓存的网络继续工作，\(remaining)到期。正在重试…"
    }
}
