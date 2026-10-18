import Foundation
import Synchronization
import os

// NSXPC delegate callbacks can arrive on arbitrary queues. Membership is locked;
// each session owns its own bounded in-flight push and cancellation state.
final class FrameServer: NSObject, NSXPCListenerDelegate, @unchecked Sendable {
    private let listener: NSXPCListener
    private let node: NodeHandle
    private struct Membership {
        var stopping = false
        var sessions: [UUID: FrameSession] = [:]
    }
    private let sessions = Mutex(Membership())

    init(node: NodeHandle) {
        self.node = node
        do {
            listener = NSXPCListener(machServiceName: try ShellConfiguration.value("MeshwayMachService"))
            super.init()
            listener.setConnectionCodeSigningRequirement(try ShellConfiguration.requirement(server: false))
            listener.delegate = self
        } catch { fatalError("Invalid signed XPC configuration: \(error)") }
    }

    func start() { listener.resume() }

    func stop() {
        listener.invalidate()
        let current = sessions.withLock { state in
            state.stopping = true
            let current = Array(state.sessions.values)
            state.sessions.removeAll()
            return current
        }
        current.forEach { $0.cancel() }
    }

    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        let id = UUID()
        let session = FrameSession(connection: connection, node: node)
        guard session.available else { session.cancel(); return false }
        let accepted = sessions.withLock { state in
            guard !state.stopping, state.sessions.count < 8 else { return false }
            state.sessions[id] = session
            return true
        }
        guard accepted else { session.cancel(); return false }
        connection.exportedInterface = NSXPCInterface(with: FrameService.self)
        connection.exportedObject = session
        connection.remoteObjectInterface = NSXPCInterface(with: FrameReceiver.self)
        let cancel: @Sendable () -> Void = { [weak self, weak session] in
            session?.cancel()
            _ = self?.sessions.withLock { $0.sessions.removeValue(forKey: id) }
        }
        connection.invalidationHandler = cancel
        connection.interruptionHandler = cancel
        connection.resume()
        return true
    }
}

final class FrameSession: NSObject, FrameService, FrameSink, @unchecked Sendable {
    private struct State {
        var cancelled = false
        var inFlight = false
        var deadline: ContinuousClock.Instant?
        var rust: Session?
    }
    private let state = Mutex(State())
    private let connection: NSXPCConnection
    private let logger = Logger(subsystem: Bundle.main.bundleIdentifier ?? "Meshway", category: "xpc-shell")
    private let timer = DispatchSource.makeTimerSource(queue: DispatchQueue(label: "meshway.xpc.push"))
    var available: Bool { state.withLock { !$0.cancelled && $0.rust != nil } }

    init(connection: NSXPCConnection, node: NodeHandle) {
        self.connection = connection
        super.init()
        do {
            let session = try node.openIpcSession(sink: self)
            state.withLock { $0.rust = session }
        } catch { logger.error("Rust IPC session unavailable") }
        timer.setEventHandler { [weak self] in self?.tick() }
        timer.schedule(deadline: .now() + 1, repeating: 1)
        timer.activate()
        logger.notice("session created peer-pid=\(connection.processIdentifier)")
    }

    func cancel() {
        let first = state.withLock { state in
            guard !state.cancelled else { return false }
            state.cancelled = true
            return true
        }
        if first {
            timer.cancel()
            connection.invalidate()
            logger.notice("session cancelled peer-pid=\(self.connection.processIdentifier)")
            let session = state.withLock { state in
                let session = state.rust
                state.rust = nil
                return session
            }
            Task { await session?.close() }
        }
    }

    func send(_ frame: Data, reply: @escaping @Sendable (Bool) -> Void) {
        guard frame.count <= ShellConfiguration.maximumFrameSize else {
            reply(false)
            cancel()
            return
        }
        dispatchFrame(frame, reply: reply)
    }

    private func dispatchFrame(_ frame: Data, reply: @escaping @Sendable (Bool) -> Void) {
        do {
            guard let session = state.withLock({ $0.cancelled ? nil : $0.rust }) else { reply(false); return }
            try session.send(frame: frame)
            reply(true)
        } catch { reply(false); cancel() }
    }

    private func tick() {
        let expired = state.withLock { $0.deadline.map { $0 < .now } ?? false }
        if expired { cancel(); return }
    }

    // UniFFI invokes this on a blocking worker, never on the Node's executor.
    // One ACK and a fixed deadline bound both XPC buffering and stop/join.
    func onFrame(frame: Data) -> Bool {
        let done = DispatchSemaphore(value: 0)
        let accepted = Mutex(false)
        push(frame) { success in accepted.withLock { $0 = success }; done.signal() }
        guard done.wait(timeout: .now() + 5) == .success else { cancel(); return false }
        return accepted.withLock { $0 }
    }

    private func push(_ frame: Data, completion: @escaping @Sendable (Bool) -> Void) {
        let accepted = state.withLock { state in
            guard !state.cancelled, !state.inFlight else { return false }
            state.inFlight = true
            state.deadline = .now + .seconds(5)
            return true
        }
        guard accepted else { completion(false); return }
        guard let remote = connection.remoteObjectProxyWithErrorHandler({ [weak self] _ in
            self?.cancel()
        }) as? FrameReceiver else { cancel(); completion(false); return }
        remote.receive(frame) { [weak self] in
            guard let self else { return }
            let active = state.withLock { state in
                state.inFlight = false
                state.deadline = nil
                return !state.cancelled
            }
            completion(active)
        }
    }
}
