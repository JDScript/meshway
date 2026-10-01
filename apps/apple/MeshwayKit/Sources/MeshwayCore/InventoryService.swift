import Foundation
import MeshwayBindings

public struct MeshDevice: Sendable, Equatable, Identifiable {
  public let id: String
  public let name: String
  public let country: String?
  public let meshIPv4: String
  public let meshIPv6: String
  public init(id: String, name: String, country: String?, meshIPv4: String, meshIPv6: String) {
    self.id = id
    self.name = name
    self.country = country
    self.meshIPv4 = meshIPv4
    self.meshIPv6 = meshIPv6
  }
}

public struct MeshInventory: Sendable, Equatable {
  public let revision: UInt32
  public let digest: String
  public let ipv4Prefix: String
  public let ipv6Prefix: String
  public let ownDevice: MeshDevice
  public let peers: [MeshDevice]
  public init(
    revision: UInt32, digest: String, ipv4Prefix: String, ipv6Prefix: String,
    ownDevice: MeshDevice, peers: [MeshDevice]
  ) {
    self.revision = revision
    self.digest = digest
    self.ipv4Prefix = ipv4Prefix
    self.ipv6Prefix = ipv6Prefix
    self.ownDevice = ownDevice
    self.peers = peers
  }
}

public struct MeshDnsName: Sendable, Equatable {
  public let nodeID: String
  public let name: String
  public let meshIPv4: String
  public let meshIPv6: String
  public init(nodeID: String, name: String, meshIPv4: String, meshIPv6: String) {
    self.nodeID = nodeID
    self.name = name
    self.meshIPv4 = meshIPv4
    self.meshIPv6 = meshIPv6
  }
}
public struct MeshDnsInventory: Sendable, Equatable {
  public let revision: UInt32
  public let digest: String
  public let zone: String?
  public let names: [MeshDnsName]
  public init(revision: UInt32, digest: String, zone: String?, names: [MeshDnsName]) {
    self.revision = revision
    self.digest = digest
    self.zone = zone
    self.names = names
  }
}

public protocol InventoryService: Sendable {
  func configuration(server: String, credential: String, nodeID: String) async throws
    -> MeshInventory
  func dns(server: String, credential: String, nodeID: String) async throws -> MeshDnsInventory
}

extension RustEnrollmentService: InventoryService {
  private func device(_ value: InventoryDevice) -> MeshDevice {
    MeshDevice(
      id: value.id, name: value.name, country: value.country,
      meshIPv4: value.meshIpv4, meshIPv6: value.meshIpv6)
  }
  public func configuration(server: String, credential: String, nodeID: String) async throws
    -> MeshInventory
  {
    do {
      let value = try await getDeviceConfiguration(
        serverUrl: server, credential: credential, nodeId: nodeID)
      return MeshInventory(
        revision: value.revision, digest: value.digest,
        ipv4Prefix: value.ipv4Prefix, ipv6Prefix: value.ipv6Prefix,
        ownDevice: device(value.ownDevice), peers: value.peers.map(device))
    } catch { throw sanitized(error) }
  }
  public func dns(server: String, credential: String, nodeID: String) async throws
    -> MeshDnsInventory
  {
    do {
      let value = try await getDeviceDns(serverUrl: server, credential: credential, nodeId: nodeID)
      return MeshDnsInventory(
        revision: value.revision, digest: value.digest, zone: value.zone,
        names: value.names.map {
          MeshDnsName(
            nodeID: $0.nodeId, name: $0.name, meshIPv4: $0.meshIpv4, meshIPv6: $0.meshIpv6)
        })
    } catch { throw sanitized(error) }
  }
}
