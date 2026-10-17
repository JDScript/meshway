import Foundation
import NetworkExtension
import os
import Synchronization

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
}
