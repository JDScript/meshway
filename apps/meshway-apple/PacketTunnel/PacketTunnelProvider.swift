import Foundation
import NetworkExtension
import os
import Synchronization
import Darwin

private final class NodeLogs: LogSink, Sendable {
    private let logger = Logger(subsystem: "dev.meshway.node", category: "node")
    func line(line: String) { logger.info("\(line, privacy: .public)") }
}

// This attachment is explicitly control-only until Apple Host is connected.
// It owns the unique Rust Node; no Swift control state or HTTP client exists.
final class PacketTunnelProvider: NEPacketTunnelProvider, @unchecked Sendable {
    private struct Lifecycle {
        var server: FrameServer?
        var node: NodeHandle?
        var task: Task<Void, Never>?
    }
    private let lifecycle = Mutex(Lifecycle())

    override func startTunnel(options: [String: NSObject]?, completionHandler: @escaping @Sendable (Error?) -> Void) {
        let configuration = (protocolConfiguration as? NETunnelProviderProtocol)?.providerConfiguration
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
                    let node = try startControlOnly(config: StartConfig(stateDirectory: path, serverOrigin: origin, hostname: nil), logs: NodeLogs())
                    lifecycle.withLock { $0.node = node }
                    try Task.checkCancellation()
                    let server = FrameServer(node: node)
                    lifecycle.withLock { $0.server = server }
                    server.start()
                    completionHandler(nil)
                } catch {
                    let node = lifecycle.withLock { state in
                        let node = state.node
                        state.node = nil
                        return node
                    }
                    try? await node?.stop()
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
                state.server = nil; state.node = nil; state.task = nil
                return owned
            }
            server?.stop()
            do { try await node?.stop() }
            catch { Logger(subsystem: "dev.meshway.node", category: "node").error("Node shutdown failed: \(String(describing: error), privacy: .public)") }
            completionHandler()
        }
    }
}
