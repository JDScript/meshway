import MeshwayIPC
import NetworkExtension

open class PacketTunnelProvider: NEPacketTunnelProvider {
  open override func startTunnel(
    options: [String: NSObject]? = nil,
    completionHandler: @escaping (Error?) -> Void
  ) {
    completionHandler(TunnelUnavailable.notImplemented)
  }

  open override func stopTunnel(
    with reason: NEProviderStopReason,
    completionHandler: @escaping () -> Void
  ) {
    completionHandler()
  }
}
