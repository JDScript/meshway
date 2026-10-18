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
    var issue: String?
    var ipcIssue: String?
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
    var title: String { status?.node?.hostname ?? (status?.identity == nil ? "Join your network" : "This Mac") }
    var phase: String { status.map { label($0.phase) } ?? "Node status unavailable" }
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
        if sampleMode { return "Sample VPN state" }
        if tunnel.preferencesError != nil { return "Status unavailable" }
        guard tunnel.preferencesLoaded else { return "Not checked" }
        guard tunnel.hasConfiguration else { return "Configuration missing" }
        switch tunnel.vpnStatus {
        case .invalid: return "Invalid configuration"
        case .disconnected: return "Disconnected"
        case .connecting: return "Connecting"
        case .connected: return "Connected"
        case .reasserting: return "Reasserting"
        case .disconnecting: return "Disconnecting"
        @unknown default: return "Unknown"
        }
    }
    func name(_ id: String) -> String { status?.peers.first { $0.nodeId == id }?.node.hostname ?? id }
    func path(_ peer: ClientPeer) -> String {
        if peer.state == .direct { return peer.endpoint ?? "Endpoint not reported" }
        if peer.state == .relay { return "via " + peer.via.map(name).joined(separator: " → ") }
        if let idle = peer.idlePath {
            switch idle.route {
            case .direct: return "Direct · idle"
            case .relay: return "Relay · idle via " + idle.via.map(name).joined(separator: " → ")
            case .none: return "No path"
            }
        }
        return "Path not reported"
    }
    func peerStateLabel(_ peer: ClientPeer) -> String {
        if peer.state == .idle || peer.state == .relayIdle, let idle = peer.idlePath {
            switch idle.route {
            case .direct: return "Direct · idle"
            case .relay: return "Relay · idle"
            case .none: break
            }
        }
        return human(peer.state)
    }
    func peerPathDetail(_ peer: ClientPeer) -> String? {
        if peer.state == .idle || peer.state == .relayIdle, let idle = peer.idlePath {
            switch idle.route {
            case .direct: return nil
            case .relay: return "via " + idle.via.map(name).joined(separator: " → ")
            case .none: break
            }
        }
        return path(peer)
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
            notice = "Routing selection applied."
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
            status = nil; receivedAt = nil; ipcIssue = error.localizedDescription
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
                    catch { issue = "Use an HTTPS Server origin without a path, query or credentials. HTTP is supported only for numeric loopback addresses."; return }
                    guard await tunnel.start(defaultRoute: false, includeAllNetworks: false, serverOrigin: server) else { issue = tunnel.message; return }
                    for _ in 0..<30 {
                        try await Task.sleep(for: .milliseconds(500))
                        await tunnel.refresh()
                        if tunnel.vpnStatus == .connected { break }
                    }
                    guard tunnel.vpnStatus == .connected else { issue = "Waiting for the VPN. Check System Settings for extension or VPN approval."; return }
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
                notice = "Down was submitted; waiting for Node cleanup before stopping the VPN."
            }
        } catch { issue = error.localizedDescription }
    }
    func perform(_ command: ClientCommand) async {
        guard !busy, !sampleMode else { return }
        busy = true; issue = nil; notice = nil
        statusRead?.cancel()
        defer { busy = false }
        do { try handle(try await IPCExchange().run(command)) }
        catch { issue = error.localizedDescription }
    }
    func identityOperation(logout: Bool) async {
        guard !busy, !sampleMode else { return }
        busy = true; issue = nil; notice = nil
        statusRead?.cancel()
        defer { busy = false }
        do {
            guard case .status(let current) = try await IPCExchange().run(.status) else { throw IPCFailure.result }
            try handle(try await IPCExchange().run(logout ? .logout(revision: current.revision) : .rotateKey(revision: current.revision)))
            if logout { notice = status?.phase == .leaving ? "Logout submitted. The Node is waiting for Server confirmation." : "Logout response received." }
        } catch { issue = error.localizedDescription }
    }
    private func handle(_ result: ClientResult) throws {
        switch result {
        case .status(let status): accept(status)
        case .error(_, let message): throw IPCFailure.remote(message)
        case .selection(let selection):
            committedSelection = selection
            notice = "Server committed the selection. Waiting for its netmap to be applied."
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
                if case .error(_, let message) = result { issue = message }
            } catch is CancellationError {} catch { issue = error.localizedDescription }
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
    case .down: "Down"
    case .starting: "Starting"
    case .awaitingApproval: "Awaiting approval"
    case .synchronizing: "Synchronizing"
    case .controlReady: "Control ready"
    case .running: "Running"
    case .retrying: "Retrying"
    case .blocked: "Blocked"
    case .stopping: "Stopping"
    case .leaving: "Leaving"
    }
}
