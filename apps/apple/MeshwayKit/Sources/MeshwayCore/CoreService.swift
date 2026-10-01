import Foundation
import MeshwayBindings

public struct CoreBuildInfo: Sendable, Equatable {
  public let version: String
  public let configurationSchema: UInt32
  public let maximumRoutingRules: UInt32
}

public protocol CoreService: Sendable {
  func buildInfo() -> CoreBuildInfo
  func isValidTagSelector(_ value: String) -> Bool
}

public struct RustCoreService: CoreService {
  public init() {}
  public func buildInfo() -> CoreBuildInfo {
    let info = coreInfo()
    return CoreBuildInfo(
      version: info.version, configurationSchema: info.configurationSchema,
      maximumRoutingRules: info.maximumRoutingRules)
  }
  public func isValidTagSelector(_ value: String) -> Bool {
    validTagSelector(value: value)
  }
}

public struct PendingAuthorization: Sendable, CustomStringConvertible, CustomDebugStringConvertible
{
  public var description: String { "PendingAuthorization(<redacted>)" }
  public var debugDescription: String { description }
  public let url: URL
  public let verifier: String
  public let state: String
  public init(url: URL, verifier: String, state: String) {
    self.url = url
    self.verifier = verifier
    self.state = state
  }
}

public struct EnrolledNode: Codable, Sendable, Equatable {
  public let id: String
  public let name: String
  public let meshIPv4: String
  public let meshIPv6: String
  public init(id: String, name: String, meshIPv4: String, meshIPv6: String) {
    self.id = id
    self.name = name
    self.meshIPv4 = meshIPv4
    self.meshIPv6 = meshIPv6
  }
}

public enum EnrollmentError: Error, LocalizedError, Sendable, Equatable {
  case invalidInput, invalidCode, credentialRejected, revoked, credentialConflict, capacity,
    transport, server
  public var errorDescription: String? {
    switch self {
    case .invalidInput:
      "Invalid server origin or device details. Use HTTPS (loopback HTTP is allowed for development)."
    case .invalidCode:
      "The code is invalid, expired, or bound differently. Retry the saved attempt before restarting."
    case .credentialRejected:
      "The controller rejected this device credential. The saved identity has been preserved."
    case .revoked:
      "This device has been revoked. Reset this device explicitly to create a new local identity."
    case .credentialConflict:
      "This credential is already bound. The identity has been preserved; do not automatically replace it."
    case .capacity: "The network cannot accept another node. Retry later."
    case .transport: "Cannot contact the server. Your saved identity and attempt are preserved."
    case .server: "The server returned an unexpected response. Your saved attempt is preserved."
    }
  }
}

public protocol EnrollmentService: Sendable {
  func credential() throws -> String
  func begin(server: String, name: String, version: String) throws -> PendingAuthorization
  func exchange(
    server: String, code: String, verifier: String, credential: String, name: String,
    version: String
  ) async throws -> EnrolledNode
  func heartbeat(server: String, credential: String) async throws -> EnrolledNode
}

public struct RustEnrollmentService: EnrollmentService {
  public init() {}
  private func sanitized(_ error: Error) -> EnrollmentError {
    guard let error = error as? SignInError else { return .server }
    switch error {
    case .InvalidInput: return .invalidInput
    case .InvalidCode: return .invalidCode
    case .CredentialRejected: return .credentialRejected
    case .Revoked: return .revoked
    case .CredentialConflict: return .credentialConflict
    case .Capacity: return .capacity
    case .Transport: return .transport
    case .Server: return .server
    }
  }
  public func credential() throws -> String {
    do { return try newNodeCredential() } catch { throw sanitized(error) }
  }
  public func begin(server: String, name: String, version: String) throws -> PendingAuthorization {
    do {
      let attempt = try beginAuthorization(serverUrl: server, name: name, appVersion: version)
      guard let url = URL(string: attempt.url) else { throw EnrollmentError.invalidInput }
      return PendingAuthorization(url: url, verifier: attempt.verifier, state: attempt.state)
    } catch { throw sanitized(error) }
  }
  private func summary(_ node: DeviceNode) -> EnrolledNode {
    EnrolledNode(id: node.id, name: node.name, meshIPv4: node.meshIpv4, meshIPv6: node.meshIpv6)
  }
  public func exchange(
    server: String, code: String, verifier: String, credential: String, name: String,
    version: String
  ) async throws -> EnrolledNode {
    do {
      return summary(
        try await exchangeDevice(
          serverUrl: server, code: code, verifier: verifier, credential: credential, name: name,
          appVersion: version))
    } catch { throw sanitized(error) }
  }
  public func heartbeat(server: String, credential: String) async throws -> EnrolledNode {
    do {
      return summary(try await MeshwayBindings.heartbeat(serverUrl: server, credential: credential))
    } catch { throw sanitized(error) }
  }
}
