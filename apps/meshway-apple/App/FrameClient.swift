import Foundation

final class FrameClient: NSObject, FrameReceiver, @unchecked Sendable {
    private let connection: NSXPCConnection
    private let output: @Sendable (Data, @escaping @Sendable () -> Void) -> Void

    init(output: @escaping @Sendable (Data, @escaping @Sendable () -> Void) -> Void, disconnected: @escaping @Sendable () -> Void) throws {
        connection = NSXPCConnection(machServiceName: try ShellConfiguration.value("MeshwayMachService"), options: .privileged)
        self.output = output
        super.init()
        connection.setCodeSigningRequirement(try ShellConfiguration.requirement(server: true))
        connection.remoteObjectInterface = NSXPCInterface(with: FrameService.self)
        connection.exportedInterface = NSXPCInterface(with: FrameReceiver.self)
        connection.exportedObject = self
        connection.invalidationHandler = disconnected
        connection.interruptionHandler = { [weak self] in self?.close() }
        connection.resume()
    }

    func receive(_ frame: Data, reply: @escaping @Sendable () -> Void) {
        guard frame.count <= ShellConfiguration.maximumFrameSize else { close(); return }
        output(frame, reply)
    }

    func send(_ frame: Data, reply: @escaping @Sendable (Bool) -> Void) {
        guard frame.count <= ShellConfiguration.maximumFrameSize,
              let proxy = connection.remoteObjectProxyWithErrorHandler({ [weak self] _ in self?.close() }) as? FrameService
        else { reply(false); return }
        proxy.send(frame, reply: reply)
    }

    func close() { connection.invalidate() }
}
