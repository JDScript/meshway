import Foundation
import MeshwayIPC
import NetworkExtension

public struct ControllerAddress: Equatable, Sendable {
  public let url: URL

  /// A local input rule, not a controller reachability or OAuth capability check.
  public init(_ text: String) throws {
    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
    guard let parts = URLComponents(string: trimmed),
      let scheme = parts.scheme?.lowercased(), ["http", "https"].contains(scheme),
      let host = parts.host, !host.isEmpty,
      parts.user == nil, parts.password == nil,
      parts.query == nil, parts.fragment == nil,
      parts.path.isEmpty || parts.path == "/",
      scheme == "https" || ["localhost", "127.0.0.1", "[::1]"].contains(host),
      parts.port.map({ (1...65535).contains($0) }) ?? true,
      let url = parts.url
    else { throw AddressError.invalid }
    self.url = url
  }
}

public enum AddressError: Error, LocalizedError {
  case invalid
  public var errorDescription: String? {
    "Enter the console HTTPS origin without a path, credentials, query, or fragment. Loopback HTTP is allowed for development."
  }
}

@MainActor public protocol TunnelController {
  var unavailableReason: String { get }
  func installedConfigurationCount() async throws -> Int
}

/// Only reads existing system preferences. Never saves a VPN configuration or
/// starts a connection in this slice. NEVPNStatus is not exposed as Mesh health.
@MainActor public final class SystemTunnelController: TunnelController {
  private let providerBundleID: String
  public init(providerBundleID: String) { self.providerBundleID = providerBundleID }
  public var unavailableReason: String {
    TunnelUnavailable.notImplemented.localizedDescription
  }
  public func installedConfigurationCount() async throws -> Int {
    let managers = try await NETunnelProviderManager.loadAllFromPreferences()
    return managers.filter {
      ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier
        == providerBundleID
    }.count
  }
}
