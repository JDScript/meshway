import Foundation
import NetworkExtension
import Network
import os
import Synchronization
import Darwin

private final class NodeLogs: LogSink, Sendable {
    private let logger = Logger(subsystem: "dev.meshway.node", category: "node")
    func line(line: String) { logger.info("\(line, privacy: .public)") }
}

// NE attachment only. Rust owns the unique Node and all control/packet policy.
final class PacketTunnelProvider: NEPacketTunnelProvider, @unchecked Sendable {
    private struct Lifecycle {
        var server: FrameServer?
        var node: NodeHandle?
        var task: Task<Void, Never>?
        var monitor: NWPathMonitor?
        var interface: String?
        var pathFingerprint: String?
    }
    private let lifecycle = Mutex(Lifecycle())

    override func startTunnel(options: [String: NSObject]?, completionHandler: @escaping @Sendable (Error?) -> Void) {
        let configuration = (protocolConfiguration as? NETunnelProviderProtocol)?.providerConfiguration
        let dnsAnchorAddress = configuration?["dnsAnchorAddress"] as? String
        guard let origin = configuration?["serverOrigin"] as? String else {
            completionHandler(CocoaError(.coderValueNotFound)); return
        }
        lifecycle.withLock { state in
            state.task = Task { [self] in
                do {
                    try Task.checkCancellation()
                    let path = "/Library/Application Support/Meshway"
                    // Node's private-storage owner independently validates the
                    // directory and all identity files before opening them.
                    if mkdir(path, 0o700) != 0 && errno != EEXIST { throw POSIXError(.EACCES) }
                    // Create NE's packet queue with no address/capture before
                    // handing it to Rust. All business settings come from Host.
                    try await setTunnelNetworkSettings(NEPacketTunnelNetworkSettings(tunnelRemoteAddress: "127.0.0.1"))
                    try Task.checkCancellation()
                    let (descriptor, name) = try utunDescriptor()
                    let node = try start(config: StartConfig(stateDirectory: path, serverOrigin: origin, hostname: nil, dnsAnchorAddress: dnsAnchorAddress), logs: NodeLogs(), settings: SettingsSink(provider: self), utunFd: descriptor, utunName: name)
                    lifecycle.withLock { $0.node = node }
                    try Task.checkCancellation()
                    let monitor = NWPathMonitor()
                    monitor.pathUpdateHandler = { [weak self] path in
                        guard let self else { return }
                        let interface = path.availableInterfaces.first { path.usesInterfaceType($0.type) && !$0.name.hasPrefix("utun") && $0.name != "lo0" }?.name
                        let gateways = path.gateways.map { String(describing: $0) }.sorted().joined(separator: ",")
                        let fingerprint = "\(path.status)|\(interface ?? "")|\(path.supportsIPv4)|\(path.supportsIPv6)|\(gateways)"
                        let (node, changed) = lifecycle.withLock { state in
                            let changed = state.pathFingerprint != fingerprint
                            state.interface = interface; state.pathFingerprint = fingerprint
                            return (state.node, changed)
                        }
                        try? node?.pathChanged(interface: interface, path: changed)
                    }
                    lifecycle.withLock { $0.monitor = monitor }
                    monitor.start(queue: DispatchQueue(label: "meshway.path"))
                    let server = FrameServer(node: node)
                    lifecycle.withLock { $0.server = server }
                    server.start()
                    completionHandler(nil)
                } catch {
                    let node = lifecycle.withLock { $0.node }
                    // startTunnel itself may have been cancelled. Cleanup is
                    // joined in an uncancelled task, including the Rust join.
                    await Task.detached { try? await node?.stop() }.value
                    lifecycle.withLock { $0.node = nil }
                    completionHandler(error)
                }
            }
        }
    }

    override func stopTunnel(with reason: NEProviderStopReason, completionHandler: @escaping @Sendable () -> Void) {
        let task = lifecycle.withLock { state in state.task?.cancel(); return state.task }
        Task {
            await task?.value
            let (server, node) = lifecycle.withLock { state in
                let owned = (state.server, state.node)
                state.monitor?.cancel()
                state.monitor = nil
                state.server = nil; state.node = nil; state.task = nil
                return owned
            }
            server?.stop()
            do { try await node?.stop() }
            catch { Logger(subsystem: "dev.meshway.node", category: "node").error("Node shutdown failed: \(String(describing: error), privacy: .public)") }
            completionHandler()
        }
    }

    override func sleep(completionHandler: @escaping @Sendable () -> Void) {
        refreshPath()
        completionHandler()
    }
    override func wake() { refreshPath() }
    private func refreshPath() {
        let (node, interface) = lifecycle.withLock { ($0.node, $0.interface) }
        try? node?.pathChanged(interface: interface, path: false)
    }
    private func utunDescriptor() throws -> (Int32, String) {
        var found: [(Int32, String)] = []
        for descriptor in 0..<min(getdtablesize(), 4096) {
            var name = [CChar](repeating: 0, count: Int(IFNAMSIZ))
            var count = socklen_t(name.count)
            guard getsockopt(descriptor, SYSPROTO_CONTROL, 2, &name, &count) == 0 else { continue }
            let interface = String(decoding: name.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
            if interface.hasPrefix("utun") { found.append((descriptor, interface)) }
        }
        guard found.count == 1 else { throw CocoaError(.fileReadUnknown) }
        return found[0]
    }
}

// The Rust callback is synchronous on its blocking executor. Serialize NE
// mutations and report completion only after NE's own completion handler.
private final class SettingsSink: TunnelSettingsSink, @unchecked Sendable {
    private weak var provider: PacketTunnelProvider?
    private let queue = DispatchQueue(label: "meshway.ne.settings")
    private let pending = Mutex(false)
    init(provider: PacketTunnelProvider) { self.provider = provider }
    func apply(settings: NetworkSettings?) throws {
        try queue.sync {
            guard let provider else { throw BridgeError.Stopped }
            let network: NEPacketTunnelNetworkSettings?
            if let settings {
                let value = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: "127.0.0.1")
                if !settings.routes4.isEmpty || settings.ipv4 != nil {
                    let addresses = settings.ipv4.map { [$0] } ?? []
                    let ipv4 = NEIPv4Settings(addresses: addresses, subnetMasks: addresses.map { _ in "255.255.255.255" })
                    ipv4.includedRoutes = settings.routes4.map { NEIPv4Route(destinationAddress: $0.address, subnetMask: $0.netmask) }
                    value.ipv4Settings = ipv4
                }
                if !settings.routes6.isEmpty || settings.ipv6 != nil {
                    let addresses = settings.ipv6.map { [$0] } ?? []
                    let ipv6 = NEIPv6Settings(addresses: addresses, networkPrefixLengths: addresses.map { _ in NSNumber(value: 128) })
                    ipv6.includedRoutes = settings.routes6.map { NEIPv6Route(destinationAddress: $0.address, networkPrefixLength: NSNumber(value: $0.prefix)) }
                    value.ipv6Settings = ipv6
                }
                value.mtu = NSNumber(value: settings.mtu)
                if let dns = settings.dns {
                    let resolver = NEDNSSettings(servers: [dns.server])
                    resolver.matchDomains = dns.matchDomains
                    resolver.searchDomains = dns.searchDomains
                    resolver.matchDomainsNoSearch = true
                    value.dnsSettings = resolver
                }
                network = value
            } else { network = nil }
            let completed = DispatchSemaphore(value: 0)
            let failure = Mutex<String?>(nil)
            try pending.withLock { pending in
                guard !pending else { throw BridgeError.Startup(message: "Previous NE settings change is still pending; capture retained") }
                pending = true
            }
            provider.setTunnelNetworkSettings(network) { [self] error in
                failure.withLock { $0 = error?.localizedDescription }
                pending.withLock { $0 = false }
                completed.signal()
            }
            guard completed.wait(timeout: .now() + 5) == .success else {
                // The OS outcome is unknown. Subsequent mutations are rejected
                // until this callback completes; Node withdraws packet output.
                throw BridgeError.Startup(message: "NE settings completion timed out; capture retained")
            }
            if let message = failure.withLock({ $0 }) { throw BridgeError.Startup(message: message) }
        }
    }
}
