import AppKit
import Foundation
import NetworkExtension
import Observation

@MainActor @Observable
final class AppModel {
    let tunnel = TunnelController()
    private(set) var status: ClientStatus?
    private(set) var receivedAt: Date?
    var server = ""
    var enrollmentKey = ""
    var search = ""
    var busy = false
    var issue: AppIssue?
    var ipcIssue: AppIssue?
    var notice: String?
    var netcheck: ClientNetcheck?
    var pingEvents: [ClientPingEvent] = []
    var pingTarget = ""
    var pingRunning = false
    var sampleMode = false
    #if DEBUG
    var previewSection: String?
    #endif
    private var openedApproval: String?
    private var polling = false
    private var pollTask: Task<Void, Never>?
    private var stopWhenDown = false
    private var statusRead: Task<ClientResult, Error>?
    private var pingTask: Task<Void, Never>?
    private var committedSelection: ClientSelection?

    var peers: [ClientPeer] {
        let query = search.trimmingCharacters(in: .whitespacesAndNewlines)
        return (status?.peers ?? []).filter { peer in
            query.isEmpty || [peer.node.hostname, peer.node.ipv4, peer.node.ipv6, peer.nodeId].compactMap { $0 }.contains { $0.localizedStandardContains(query) }
        }
    }
    var phase: String { status.map { label($0.phase) } ?? String(localized: "Unavailable") }
    var selection: ClientSelection? { status?.netmap?.routing }
    // Presentation readiness only. The Node still validates each request and CAS.
    var canRequestSelection: Bool {
        guard let status else { return false }
        return status.desiredUp && status.identity != nil && status.netmap != nil && status.instance != nil
    }
    var canRotateKey: Bool { status?.identity != nil && status?.phase != .leaving }
    var canLogout: Bool { status?.identity != nil }
    var netmapApplied: Bool {
        guard let reference = status?.netmap?.reference, let applied = status?.applied else { return false }
        return reference == applied
    }
    var vpnLabel: String {
        if sampleMode { return String(localized: "Preview") }
        if tunnel.preferencesError != nil { return String(localized: "Unavailable") }
        guard tunnel.preferencesLoaded else { return String(localized: "Not checked") }
        guard tunnel.hasConfiguration else { return String(localized: "Not set up") }
        switch tunnel.vpnStatus {
        case .invalid: return String(localized: "Invalid")
        case .disconnected: return String(localized: "Disconnected")
        case .connecting: return String(localized: "Connecting")
        case .connected: return String(localized: "Connected")
        case .reasserting: return String(localized: "Reconnecting")
        case .disconnecting: return String(localized: "Disconnecting")
        @unknown default: return String(localized: "Unknown")
        }
    }
    func name(_ id: String) -> String { status?.peers.first { $0.nodeId == id }?.node.hostname ?? id }
    func via(_ ids: [String]) -> String { String(localized: "Via \(ids.map(name).joined(separator: " → "))") }
    func path(_ peer: ClientPeer) -> String {
        if peer.state == .direct { return peer.endpoint ?? String(localized: "Direct") }
        if peer.state == .relay { return via(peer.via) }
        if let idle = peer.idlePath {
            switch idle.route {
            case .direct: return String(localized: "Direct · idle")
            case .relay: return String(localized: "Relay · idle") + " · " + via(idle.via)
            case .none: return String(localized: "No route")
            }
        }
        return peerStateLabel(peer)
    }
    func peerStateLabel(_ peer: ClientPeer) -> String {
        if peer.state == .idle || peer.state == .relayIdle, let idle = peer.idlePath {
            switch idle.route {
            case .direct: return String(localized: "Direct · idle")
            case .relay: return String(localized: "Relay · idle")
            case .none: break
            }
        }
        return peerStateName(peer.state)
    }
    func accept(_ status: ClientStatus) {
        self.status = status
        receivedAt = Date()
        if stopWhenDown && status.phase == .down {
            stopWhenDown = false
            tunnel.stop()
        }
        if server.isEmpty { server = status.server }
        if let committed = committedSelection,
           let local = selection, committed.version == local.version,
           committed.requestedProfile == local.requestedProfile,
           netmapApplied {
            notice = String(localized: "Exit profile switched.")
            committedSelection = nil
        }
        if !sampleMode, status.phase == .awaitingApproval, let link = status.enrollmentUrl,
           openedApproval != link, let url = approvalURL(link) {
            openedApproval = link
            NSWorkspace.shared.open(url)
        }
    }
    func approvalURL(_ text: String) -> URL? {
        guard let url = URL(string: text), ["https", "http"].contains(url.scheme?.lowercased() ?? ""), url.host != nil else { return nil }
        return url
    }
    func startPolling() {
        guard !sampleMode, pollTask == nil else { return }
        pollTask = Task { await poll() }
    }
    private func poll() async {
        guard !sampleMode, !polling else { return }
        polling = true
        defer { polling = false }
        while !Task.isCancelled {
            await tunnel.refresh()
            if server.isEmpty, let origin = tunnel.serverOrigin { server = origin }
            if tunnel.vpnStatus == .connected || tunnel.vpnStatus == .reasserting {
                if !busy { await refresh() }
            } else { status = nil; receivedAt = nil; ipcIssue = nil }
            do { try await Task.sleep(for: .seconds(1)) } catch { break }
        }
    }
    func refresh() async {
        guard !sampleMode, !busy, statusRead == nil else { return }
        let read = Task { try await IPCExchange().run(.status) }
        statusRead = read
        defer { statusRead = nil }
        do {
            let result = try await read.value
            guard !read.isCancelled else { return }
            guard case .status(let status) = result else { throw IPCFailure.result }
            accept(status)
            ipcIssue = nil
        } catch {
            guard !read.isCancelled else { return }
            status = nil; receivedAt = nil; ipcIssue = AppIssue(error: error)
        }
    }
    func connect(_ enabled: Bool) async {
        guard !busy, !sampleMode else { return }
        busy = true; issue = nil; notice = nil
        statusRead?.cancel()
        if enabled { stopWhenDown = false }
        defer { busy = false }
        do {
            if enabled {
                await tunnel.refresh()
                if tunnel.vpnStatus != .connected && tunnel.vpnStatus != .reasserting {
                    do { server = try clientServerOrigin(value: server) }
                    catch { issue = AppIssue(.serverURLInvalid); return }
                    guard await tunnel.start(defaultRoute: false, includeAllNetworks: false, serverOrigin: server) else { issue = AppIssue(.vpnStartFailed, detail: tunnel.message); return }
                    for _ in 0..<30 {
                        try await Task.sleep(for: .milliseconds(500))
                        await tunnel.refresh()
                        if tunnel.vpnStatus == .connected { break }
                    }
                    guard tunnel.vpnStatus == .connected else { issue = AppIssue(.vpnStartTimeout); return }
                }
            }
            let response = try await IPCExchange().run(.status)
            guard case .status(let current) = response else { throw IPCFailure.result }
            let key = enrollmentKey.isEmpty ? nil : enrollmentKey
            enrollmentKey = ""
            let command: ClientCommand = enabled ? .up(revision: current.revision, enrollmentKey: key, reenroll: false) : .down(revision: current.revision)
            try handle(try await IPCExchange().run(command))
            if !enabled {
                stopWhenDown = true
                // Down's response can still be stopping. Keep the extension alive
                // until its real cleanup finishes; do not race it with stopTunnel.
                for _ in 0..<30 {
                    if status?.phase == .down { stopWhenDown = false; tunnel.stop(); return }
                    try await Task.sleep(for: .seconds(1))
                    try handle(try await IPCExchange().run(.status))
                }
                notice = String(localized: "Disconnecting…")
            }
        } catch { issue = AppIssue(error: error) }
    }
    func perform(_ command: ClientCommand) async {
        guard !busy, !sampleMode else { return }
        busy = true; issue = nil; notice = nil
        statusRead?.cancel()
        defer { busy = false }
        do { try handle(try await IPCExchange().run(command)) }
        catch { issue = AppIssue(error: error) }
    }
    func identityOperation(logout: Bool) async {
        guard !busy, !sampleMode else { return }
        busy = true; issue = nil; notice = nil
        statusRead?.cancel()
        defer { busy = false }
        do {
            guard case .status(let current) = try await IPCExchange().run(.status) else { throw IPCFailure.result }
            try handle(try await IPCExchange().run(logout ? .logout(revision: current.revision) : .rotateKey(revision: current.revision)))
            if logout { notice = status?.phase == .leaving ? String(localized: "Signing out; waiting for the server.") : String(localized: "Signed out.") }
        } catch { issue = AppIssue(error: error) }
    }
    private func handle(_ result: ClientResult) throws {
        switch result {
        case .status(let status): accept(status)
        case .error(let code, let message): throw IPCFailure.remote(code, message)
        case .selection(let selection):
            committedSelection = selection
            notice = String(localized: "Switching exit profile…")
        case .netcheck(let report): netcheck = report
        case .ping: throw IPCFailure.result
        }
    }
    func ping() {
        guard !pingRunning, !sampleMode, !pingTarget.isEmpty else { return }
        pingRunning = true; pingEvents = []; issue = nil
        pingTask = Task {
            defer { pingRunning = false; pingTask = nil }
            do {
                let result = try await IPCExchange().run(.ping(target: pingTarget, count: 4)) { event in
                    self.recordPing(event)
                }
                if case .error(let code, let message) = result { issue = AppIssue(.ipc(code), detail: message) }
            } catch is CancellationError {} catch { issue = AppIssue(error: error) }
        }
    }
    func cancelPing() { pingTask?.cancel() }
    private func recordPing(_ event: ClientPingEvent) {
        if case .sample(let observation, let measurement, let version, _, _, _, _) = event,
           let index = pingEvents.firstIndex(where: {
               if case .sample(let oldObservation, let oldMeasurement, _, _, _, _, _) = $0 { return observation == oldObservation && measurement == oldMeasurement }
               return false
           }), case .sample(_, _, let previousVersion, _, _, _, _) = pingEvents[index] {
            if version > previousVersion { pingEvents[index] = event }
            return
        }
        if pingEvents.count == 128 { pingEvents.removeFirst() }
        pingEvents.append(event)
    }
}

func label(_ value: ClientPhase) -> String {
    switch value {
    case .down: String(localized: "Off")
    case .starting: String(localized: "Starting")
    case .awaitingApproval: String(localized: "Waiting for approval")
    case .synchronizing: String(localized: "Syncing")
    case .controlReady: String(localized: "Applying settings")
    case .running: String(localized: "Running")
    case .retrying: String(localized: "Retrying")
    case .blocked: String(localized: "Stopped by an error")
    case .stopping: String(localized: "Stopping")
    case .leaving: String(localized: "Signing out")
    }
}
