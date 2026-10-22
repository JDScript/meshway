import SwiftUI

enum EmptyStateKind: Equatable {
    case devices, search(String), mesh, profiles, serverUnavailable, serverCached(Date)
    var title: String {
        switch self {
        case .devices: String(localized: "No other devices yet")
        case .search(let query): String(localized: "No devices match “\(query)”")
        case .mesh: String(localized: "Mesh only")
        case .profiles: String(localized: "No exit profiles")
        case .serverUnavailable, .serverCached: String(localized: "Can’t reach the server")
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
        case .devices: String(localized: "Devices that join this network appear here.")
        case .search: String(localized: "Try part of a name or IP address.")
        case .mesh: String(localized: "Your admin can add more profiles.")
        case .profiles: String(localized: "Profiles appear once you’re connected.")
        case .serverUnavailable: String(localized: "Retrying…")
        case .serverCached: String(localized: "Working offline. Retrying…")
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
    var height: CGFloat?
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
            .frame(maxWidth: .infinity).frame(height: height ?? (compact ? 132 : 196))
    }
}

struct EmptyStateView: View {
    let kind: EmptyStateKind
    var compact = false
    var height: CGFloat?
    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            QuietEmptyState(symbol: kind.symbol, title: kind.title, explanation: explanation(now: context.date), compact: compact, height: height)
        }
    }
    private func explanation(now: Date) -> String {
        guard case .serverCached(let expiry) = kind else { return kind.explanation }
        guard expiry > now else { return String(localized: "Offline access has ended. Retrying…") }
        return offlineText(expiry)
    }
}
