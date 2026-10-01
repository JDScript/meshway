import Foundation

/// Reserved app↔provider message contract. No message transport is wired yet.
public enum ProviderRequest: String, Codable, Sendable {
  case diagnostics
}

public enum TunnelUnavailable: String, Error, LocalizedError, Codable, Sendable {
  case notImplemented
  public var errorDescription: String? {
    "Packet Tunnel is not implemented in this build. No network settings or packet forwarding were installed."
  }
}
