import Foundation
import NetworkExtension
import os
import Synchronization
import Darwin

// NetworkExtension owns the provider lifecycle. Only the locked task/server slot
// crosses callbacks; each settings object and URLSession is local to its task.
final class PacketTunnelProvider: NEPacketTunnelProvider, @unchecked Sendable {
    private struct Lifecycle {
        var server: FrameServer?
        var task: Task<Void, Never>?
    }
    private let lifecycle = Mutex(Lifecycle())
    private let logger = Logger(subsystem: Bundle.main.bundleIdentifier ?? "Meshway", category: "shell")

    override func startTunnel(options: [String: NSObject]?, completionHandler: @escaping @Sendable (Error?) -> Void) {
        let fullRoute = (protocolConfiguration as? NETunnelProviderProtocol)?.providerConfiguration?["labDefaultRoute"] as? Bool ?? false
        // Both probes originate inside the extension. No DNS or forwarding is installed.
        lifecycle.withLock { state in
            state.task = Task { [self] in
                await probe(label: "before-settings")
                do {
                    try Task.checkCancellation()
                    let settings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: "10.203.0.1")
                    let ipv4 = NEIPv4Settings(addresses: ["10.203.0.2"], subnetMasks: ["255.255.255.0"])
                    ipv4.includedRoutes = fullRoute ? [.default()] : [NEIPv4Route(destinationAddress: "10.203.0.0", subnetMask: "255.255.255.0")]
                    settings.ipv4Settings = ipv4
                    settings.mtu = 1280
                    try await setTunnelNetworkSettings(settings)
                    try Task.checkCancellation()
                    observeUtunDescriptors()
                    let server = FrameServer()
                    lifecycle.withLock { $0.server = server }
                    server.start()
                    completionHandler(nil)
                    await probe(label: "after-settings")
                } catch { completionHandler(error) }
            }
        }
    }

    override func stopTunnel(with reason: NEProviderStopReason, completionHandler: @escaping @Sendable () -> Void) {
        let task = lifecycle.withLock { state in
            state.task?.cancel()
            return state.task
        }
        Task {
            await task?.value
            let server = lifecycle.withLock { state in
                let server = state.server
                state.server = nil
                state.task = nil
                return server
            }
            server?.stop()
            completionHandler()
        }
    }

    private func probe(label: String) async {
        guard let url = URL(string: "https://1.1.1.1/cdn-cgi/trace") else { return }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 8
        configuration.timeoutIntervalForResource = 10
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        do {
            let (_, response) = try await session.data(from: url)
            logger.notice("underlay-probe \(label, privacy: .public) HTTP \((response as? HTTPURLResponse)?.statusCode ?? -1)")
        } catch {
            logger.error("underlay-probe \(label, privacy: .public) error \(String(describing: error), privacy: .public)")
        }
    }

    // Observation only: no fd ownership transfer, duplicate, read or write.
    private func observeUtunDescriptors() {
        var found = 0
        let limit = min(getdtablesize(), 4096)
        for fd in 0..<limit {
            var name = [CChar](repeating: 0, count: Int(IFNAMSIZ))
            var nameSize = socklen_t(name.count)
            // <net/if_utun.h>: UTUN_OPT_IFNAME = 2, level SYSPROTO_CONTROL.
            // Kernel control sockets need not expose AF_SYSTEM via getsockname;
            // query the utun option itself and validate the returned interface name.
            guard getsockopt(fd, SYSPROTO_CONTROL, 2, &name, &nameSize) == 0 else { continue }
            let interface = String(decoding: name.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
            guard interface.hasPrefix("utun") else { continue }
            found += 1
            logger.notice("utun-observation pid=\(getpid()) uid=\(getuid()) fd=\(fd) interface=\(interface, privacy: .public)")
        }
        logger.notice("utun-observation scanned=\(limit) found=\(found)")
    }
}
