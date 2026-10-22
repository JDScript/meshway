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
                precondition(model.path(model.peers[0]) == model.via([model.peers[0].via[0]]) && model.path(model.peers[0]).contains("meshway-tokyo"))
                model.search = "HOME"
                precondition(model.path(model.peers[0]) == String(localized: "Direct · idle"))
                precondition(model.peerStateLabel(model.peers[0]) == String(localized: "Direct · idle"))
                precondition(model.canRequestSelection && model.canRotateKey && model.canLogout)
                precondition(model.selection?.requestedProfile == "work")
                precondition(model.selection?.appliedProfile == "work")
                precondition(model.netmapApplied)
                precondition(ConnectionPresentation(model: model).state == .connected)
                let pending = try await PreviewMode.model(.awaitingApproval)
                precondition(pending.phase == label(.awaitingApproval))
                precondition(ConnectionPresentation(model: pending).state == .approval)
                precondition(pending.status?.enrollmentUrl != nil)
                precondition(pending.sampleMode, "Sample approval must never open the browser")
                precondition(pending.approvalURL("file:///etc/passwd") == nil)
                let failed = try await PreviewMode.model(.error)
                precondition(failed.status?.lastError == .credentials)
                precondition(ConnectionPresentation(model: failed).errors.map(\.code) == [.credentialRejected, .dnsSetup])
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
                registryTests()
                motionTests()
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
        let cachedPresentation = ConnectionPresentation(model: cached)
        precondition(cachedPresentation.showsWarning && cachedPresentation.warnings.map(\.code) == [.syncServerUnreachable])
        precondition(cachedPresentation.errors.isEmpty && cachedPresentation.offlineUntil != nil)
        let quiet = try await PreviewMode.model(.running)
        precondition(!ConnectionPresentation(model: quiet).showsWarning)
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
        precondition(PingPresentation(events: denied.pingEvents).failure?.code == .routeNotAllowed)
        try await PreviewMode.load("netcheck", into: running)
        guard let report = running.netcheck else { preconditionFailure("Netcheck sample decodes") }
        let facts = NetcheckPresentation(report: report).facts
        precondition(facts.first(where: { $0.id == "IPv6" })?.value == String(localized: "Not detected"))
        precondition(facts.first(where: { $0.id == "nat:0" })?.value == String(localized: "Varies by destination · direct links are harder"))
        precondition(facts.first(where: { $0.id == "nat:0" })?.title == String(localized: "NAT mapping"), "The local socket is help text, not the title")
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
        precondition(PingPresentation(events: [.failed(code: .unsupported)]).failure?.code == .pingUnsupported)
        let decision = ClientDecision(destination: nil, node: nil, resolution: nil, profile: nil, rule: nil, action: .denied(reason: .noPath), path: [])
        precondition(PingPresentation(events: [.decision(decision: decision), .failed(code: .permissionDenied)]).failure?.code == .routeNoPath)
        let unavailable = ClientNetcheck(observedAt: now, server: "https://mesh.example", serverReachable: nil, serverRttUs: nil, unavailable: .notRunning, ipv4: false, ipv6: false, udp: false, sockets: [], observers: [])
        precondition(NetcheckPresentation(report: unavailable).facts.first(where: { $0.id == "UDP" })?.value == String(localized: "Not tested"))
        precondition(NetcheckPresentation(report: unavailable).issue?.code == .netcheckNotConnected)
        print("PASS presentation: connection steps, cache lease boundaries, empty states, RTT/setup/loss and missing netcheck")
    }
    private static func registryTests() {
        func table(_ language: String) -> [String: String] {
            guard let url = Bundle.main.url(forResource: "Localizable", withExtension: "strings", subdirectory: nil, localization: language),
                  let values = NSDictionary(contentsOf: url) as? [String: String] else { preconditionFailure("Missing \(language) strings") }
            return values
        }
        let english = table("en"), chinese = table("zh-Hans")
        precondition(Set(IssueCode.allCases.map(\.slug)).count == IssueCode.allCases.count, "Slugs are unique")
        for code in IssueCode.allCases {
            precondition(code.rawValue.range(of: "^(ERROR|WARN)_[A-Z0-9_]+$", options: .regularExpression) != nil, code.rawValue)
            guard let en = english[code.rawValue], let zh = chinese[code.rawValue], !en.isEmpty, !zh.isEmpty, en != zh, en != code.rawValue
            else { preconditionFailure("\(code.rawValue) needs English and Chinese text") }
        }
        let sources = IssueSources.all
        precondition(Set(IssueCode.allCases).isSubset(of: Set(sources.map(\.code))), "Every code has a source")
        precondition(IssueSources.problems.count == 17 && IssueSources.syncErrors.count == 9 && IssueSources.ipcCodes.count == 17)
        precondition(IssueSources.routes.count == 9 && IssueSources.pings.count == 9 && IssueSources.artifacts.count == 5)
        precondition(IssueCode.sync(.network) == .syncServerUnreachable && IssueCode.syncServerUnreachable.isWarning)
        precondition(IssueCode.ping(.unsupported).rawValue == "ERROR_PING_UNSUPPORTED")
        precondition(IssueCode.problem(.credentials).rawValue == "ERROR_CREDENTIAL_REJECTED")
        print("PASS issue registry: \(IssueCode.allCases.count) unique codes, \(sources.count) mapped sources, en and zh-Hans text")
    }
    @MainActor private static func motionTests() {
        precondition(SkeletonRows.opacity(row: 0, time: nil) == SkeletonRows.opacity(row: 3, time: nil), "Reduce Motion is static")
        precondition(MeshLoader.level(node: 0, time: nil) == 1 && MeshLoader.level(node: 2, time: nil) == 1)
        let samples = stride(from: 0.0, to: Breath.period, by: 0.05)
        let bars = samples.map { SkeletonRows.opacity(row: 0, time: $0) }
        precondition(bars.min()! >= 0.45 && bars.max()! <= 0.8 && bars.max()! - bars.min()! > 0.3, "Each bar breathes between 0.45 and 0.8")
        precondition(SkeletonRows.opacity(row: 0, time: 0.4) != SkeletonRows.opacity(row: 1, time: 0.4), "Rows are staggered")
        precondition(Set([0, 1, 2].map { MeshLoader.level(node: $0, time: 0.3) }).count == 3, "Nodes light in turn")
        print("PASS motion: breathing skeleton, staggered rows, three-node loader and static Reduce Motion branches")
    }
    /// `--issue-codes`: the registry as JSON for documentation; no UI or IPC.
    static func exportCodes() {
        func table(_ language: String) -> [String: String] {
            Bundle.main.url(forResource: "Localizable", withExtension: "strings", subdirectory: nil, localization: language)
                .flatMap { NSDictionary(contentsOf: $0) as? [String: String] } ?? [:]
        }
        let english = table("en"), chinese = table("zh-Hans")
        let sources = IssueSources.all
        let values: [[String: Any]] = IssueCode.allCases.map { code in
            ["code": code.rawValue, "level": code.isWarning ? "warning" : "error", "slug": code.slug,
             "en": english[code.rawValue] ?? "", "zh-Hans": chinese[code.rawValue] ?? "",
             "sources": sources.filter { $0.code == code }.map(\.source)]
        }
        let data = try! JSONSerialization.data(withJSONObject: values, options: [.prettyPrinted, .sortedKeys])
        print(String(decoding: data, as: UTF8.self))
        exit(0)
    }
}
#endif
