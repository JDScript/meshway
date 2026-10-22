#if DEBUG
import Foundation
import NetworkExtension

// Executable model tests share the exact generated bindings linked into the App.
// --model-tests exits before NSApplication, XPC, preferences or Node startup.
enum ModelTests {
    @MainActor static func run() {
        Task { @MainActor in
            do {
                let model = try await PreviewMode.model(.running)
                precondition(model.status?.phase == .running)
                precondition(model.peers.count == 6)
                model.search = "100.64.0.4"
                precondition(model.peers.count == 1)
                precondition(model.peers.first?.rtt == nil, "Unmeasured RTT must remain nil")
                precondition(model.path(model.peers[0]) == "via meshway-tokyo")
                model.search = "HOME"
                precondition(model.path(model.peers[0]) == "Direct · idle")
                precondition(model.peerStateLabel(model.peers[0]) == "Direct · idle")
                precondition(model.peerPathDetail(model.peers[0]) == nil)
                precondition(model.canRequestSelection && model.canRotateKey && model.canLogout)
                precondition(model.selection?.requestedProfile == "work")
                precondition(model.selection?.appliedProfile == "work")
                precondition(model.netmapApplied)
                precondition(ConnectionPresentation(model: model).state == .connected)
                let pending = try await PreviewMode.model(.awaitingApproval)
                precondition(pending.phase == "Awaiting approval")
                precondition(ConnectionPresentation(model: pending).state == .approval)
                precondition(pending.status?.enrollmentUrl != nil)
                precondition(pending.sampleMode, "Sample approval must never open the browser")
                precondition(pending.approvalURL("file:///etc/passwd") == nil)
                let failed = try await PreviewMode.model(.error)
                precondition(failed.status?.lastError == .credentials)
                precondition(ConnectionPresentation(model: failed).state == .error)
                precondition(ConnectionPresentation(model: failed).errors.count == 2, "Control and network errors remain independent")
                precondition(failed.status?.networkError?.problem == .dns)
                precondition(failed.status?.node == nil && failed.peers.isEmpty)
                precondition(!failed.netmapApplied)
                precondition(!failed.canRequestSelection)
                let down = try await PreviewMode.model(.unenrolled)
                precondition(down.status?.node == nil)
                precondition(down.status?.lastSyncAt == nil)
                precondition(down.status?.netmap == nil)
                precondition(!down.canRequestSelection && !down.canRotateKey && !down.canLogout)
                precondition(ConnectionPresentation(model: down).state == .disconnected)
                for phase in [ClientPhase.starting, .synchronizing, .controlReady] {
                    precondition(IslandState.resolve(phase: phase, vpn: .connected, hasError: false, approvalRequired: false) == .connecting)
                }
                for phase in [ClientPhase.retrying, .blocked] {
                    precondition(IslandState.resolve(phase: phase, vpn: .connected, hasError: false, approvalRequired: false) == .error)
                }
                precondition(IslandState.resolve(phase: nil, vpn: .connected, hasError: false, approvalRequired: false) == .connecting, "NE connected alone does not mean Node running")
                precondition(IslandState.resolve(phase: nil, vpn: nil, hasError: false, approvalRequired: false) == .unknown)
                precondition(IslandState.resolve(phase: nil, vpn: .disconnected, hasError: false, approvalRequired: true) == .approval)
                precondition(IslandState.resolve(phase: .running, vpn: .connected, hasError: true, approvalRequired: false) == .error)
                precondition(IslandState.resolve(phase: .blocked, vpn: .connected, hasError: false, approvalRequired: true) == .error)
                precondition(IslandState.resolve(phase: .stopping, vpn: .connected, hasError: false, approvalRequired: false) == .stopping)
                precondition(IslandState.resolve(phase: .leaving, vpn: .connected, hasError: false, approvalRequired: false) == .leaving)
                try await sampleStateTests()
                presentationTests()
                print("PASS Swift model: codec fixtures, search, idle/relay paths, missing RTT, selection, approval, independent errors")
                exit(0)
            } catch { print("FAIL Swift model: \(error)"); exit(1) }
        }
        dispatchMain()
    }
    @MainActor private static func sampleStateTests() async throws {
        let steps: [(ClientSample, ConnectionStep)] = [(.connectingAuthentication, .authenticate), (.connectingSync, .synchronize),
                                                      (.connectingApply, .apply), (.connectingPlane, .apply)]
        for (sample, step) in steps {
            let model = try await PreviewMode.model(sample)
            precondition(ConnectionPresentation(model: model).state == .connecting)
            precondition(ConnectionPresentation(model: model).step == step)
        }
        let empty = try await PreviewMode.model(.noPeers)
        precondition(ConnectionPresentation(model: empty).state == .connected)
        precondition(EmptyStateKind.devices(query: "", count: empty.peers.count, server: serverEmptyState(empty)) == .devices)
        let mesh = try await PreviewMode.model(.meshOnly)
        precondition(mesh.selection?.profiles.count == 1 && mesh.selection?.profiles.first?.kind == .mesh)
        let cached = try await PreviewMode.model(.serverUnavailable)
        precondition(ConnectionPresentation(model: cached).state == .connected, "A failed sync inside the lease is not an error")
        guard case .serverCached = serverEmptyState(cached) else { preconditionFailure("Lease-backed netmap remains usable") }
        precondition(EmptyStateKind.devices(query: "", count: cached.peers.count, server: serverEmptyState(cached)) == nil)
        let expired = try await PreviewMode.model(.leaseUnavailable)
        precondition(serverEmptyState(expired) == .serverUnavailable)
        precondition(ConnectionPresentation(model: expired).state == .error)
        let running = try await PreviewMode.model(.running)
        try await PreviewMode.load("ping", into: running)
        let ping = PingPresentation(events: running.pingEvents)
        precondition(ping.decision?.action == .mesh && ping.decision?.path.count == 3)
        precondition(ping.readings.count == 4 && ping.readings[2].duration == nil && ping.readings[2].timedOut)
        precondition(ping.counts?.sent == 4 && ping.counts?.success == 3 && ping.counts?.timeout == 1)
        precondition(ping.failure == nil)
        let exit = try await PreviewMode.model(.running)
        try await PreviewMode.load("ping-exit", into: exit)
        let echoes = PingPresentation(events: exit.pingEvents)
        precondition(echoes.readings.first?.setup == true && echoes.durations.count == 2, "Setup sample stays out of RTT summary")
        let denied = try await PreviewMode.model(.running)
        try await PreviewMode.load("ping-denied", into: denied)
        precondition(PingPresentation(events: denied.pingEvents).failure == "策略拒绝：未获业务授权")
        try await PreviewMode.load("netcheck", into: running)
        guard let report = running.netcheck else { preconditionFailure("Netcheck sample decodes") }
        let facts = NetcheckPresentation(report: report).facts
        precondition(facts.first(where: { $0.id == "IPv6" })?.value == "未检测到")
        precondition(facts.first(where: { $0.id == "nat:0" })?.value == "随目的地变化 · 难以直连")
        print("PASS codec samples: connecting steps, empty/cached/expired states, ping decision/loss/setup and netcheck")
    }
    private static func presentationTests() {
        precondition(ConnectionStep.resolve(phase: .starting, plane: .stopped) == .authenticate)
        precondition(ConnectionStep.resolve(phase: .synchronizing, plane: .preparing) == .synchronize)
        precondition(ConnectionStep.resolve(phase: .controlReady, plane: .preparing) == .apply)
        precondition(ConnectionStep.resolve(phase: .running, plane: .preparing) == .apply)
        precondition(ConnectionStep.resolve(phase: .running, plane: .ready) == .ready)
        precondition(ConnectionStep.resolve(phase: .running, plane: .suspended) == nil)
        precondition(IslandState.resolve(phase: .running, vpn: .connected, hasError: false, approvalRequired: false, plane: .preparing) == .connecting)
        precondition(IslandState.resolve(phase: .running, vpn: .connected, hasError: false, approvalRequired: false, plane: .suspended) == .error)
        let now = Date()
        let expiry = now.addingTimeInterval(60)
        precondition(EmptyStateKind.server(sync: .reconnecting, issue: .network, ready: true, applied: true, expires: expiry, now: now) == .serverCached(expiry))
        precondition(EmptyStateKind.server(sync: .reconnecting, issue: .network, ready: true, applied: false, expires: expiry, now: now) == .serverUnavailable)
        precondition(EmptyStateKind.server(sync: .reconnecting, issue: .credentials, ready: true, applied: true, expires: expiry, now: now) == nil)
        precondition(EmptyStateKind.server(sync: .disconnected, issue: nil, ready: true, applied: true, expires: now, now: now) == .serverUnavailable)
        precondition(EmptyStateKind.devices(query: "missing", count: 0, server: .serverUnavailable) == .search("missing"))
        precondition(EmptyStateKind.devices(query: "", count: 0, server: nil) == .devices)
        precondition(EmptyStateKind.devices(query: "", count: 1, server: .serverUnavailable) == nil)
        // Unit inputs test the projection; they are never offered as codec preview fixtures.
        func echo(_ sequence: UInt16, _ outcome: ClientEchoOutcome, _ duration: UInt64?, setup: Bool = false) -> ClientPingEvent {
            .echo(echo: ClientEcho(sequence: sequence, outcome: outcome, from: nil, error: nil, durationUs: duration, setup: setup, path: []))
        }
        let measurements = PingPresentation(events: [echo(1, .reply, 1000), echo(2, .reply, 3000), echo(3, .timeout, nil), echo(4, .reply, 100000, setup: true)])
        precondition(measurements.median == 2)
        precondition(measurements.readings.count == 4 && measurements.readings[2].duration == nil)
        precondition(measurements.counts?.timeout == 1 && measurements.counts?.success == 3)
        precondition(PingPresentation(events: []).median == nil && PingPresentation(events: []).counts == nil)
        precondition(PingPresentation(events: [echo(1, .reply, 1000), echo(1, .reply, 2000)]).readings.count == 1)
        precondition(PingPresentation(events: [.failed(code: .unsupported)]).failure == "macOS 版暂不支持直连外部目标的 ping")
        let decision = ClientDecision(destination: nil, node: nil, resolution: nil, profile: nil, rule: nil, action: .denied(reason: .noPath), path: [])
        precondition(PingPresentation(events: [.decision(decision: decision), .failed(code: .permissionDenied)]).failure == "策略拒绝：没有可用路径")
        let unavailable = ClientNetcheck(observedAt: now, server: "https://mesh.example", serverReachable: nil, serverRttUs: nil, unavailable: .notRunning, ipv4: false, ipv6: false, udp: false, sockets: [], observers: [])
        precondition(NetcheckPresentation(report: unavailable).facts.first(where: { $0.id == "UDP" })?.value == "未测")
        print("PASS presentation: connection steps, cache lease boundaries, empty states, RTT/setup/loss and missing netcheck")
    }
}
#endif
