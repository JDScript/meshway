import Foundation
import MeshwayCore
import Testing

@testable import MeshwayModel

private let own = MeshDevice(
  id: String(repeating: "a", count: 64), name: "Mac", country: nil,
  meshIPv4: "100.64.0.1", meshIPv6: "fd42::1")
private struct InventoryEnrollment: EnrollmentService {
  func credential() throws -> String { String(repeating: "b", count: 64) }
  func begin(server: String, name: String, version: String) throws -> PendingAuthorization {
    throw EnrollmentError.server
  }
  func exchange(
    server: String, code: String, verifier: String, credential: String, name: String,
    version: String
  ) async throws -> EnrolledNode { throw EnrollmentError.server }
  func heartbeat(server: String, credential: String) async throws -> EnrolledNode {
    EnrolledNode(id: own.id, name: own.name, meshIPv4: own.meshIPv4, meshIPv6: own.meshIPv6)
  }
}
private actor InventoryFixture: InventoryService {
  var calls = 0
  var revision: UInt32 = 2
  var error: EnrollmentError?
  var dnsRevision: UInt32 = 2
  func update(revision: UInt32 = 2, dnsRevision: UInt32 = 2, error: EnrollmentError? = nil) {
    self.revision = revision
    self.dnsRevision = dnsRevision
    self.error = error
  }
  func configuration(server: String, credential: String, nodeID: String) async throws
    -> MeshInventory
  {
    calls += 1
    try await Task.sleep(for: .milliseconds(15))
    if let error { throw error }
    return MeshInventory(
      revision: revision, digest: "revision-\(revision)", ipv4Prefix: "100.64.0.0/24",
      ipv6Prefix: "fd42::/64", ownDevice: own, peers: [])
  }
  func dns(server: String, credential: String, nodeID: String) async throws -> MeshDnsInventory {
    MeshDnsInventory(
      revision: dnsRevision, digest: "dns", zone: "mesh.example",
      names: [
        MeshDnsName(
          nodeID: own.id, name: "mac.mesh.example", meshIPv4: own.meshIPv4, meshIPv6: own.meshIPv6)
      ])
  }
}

@Test @MainActor func inventoryRefreshIsReadOnlyNonOverlappingAndRevisionBound() async throws {
  let secrets = MemorySecrets()
  let snapshots = MemorySnapshots()
  let identity = IdentityJournal(
    credential: String(repeating: "b", count: 64),
    snapshot: EnrollmentSnapshot(
      server: "https://console.test", phase: .enrolled,
      node: EnrolledNode(id: own.id, name: own.name, meshIPv4: own.meshIPv4, meshIPv6: own.meshIPv6)
    ), pending: nil)
  try IdentityPersistence(secrets: secrets, snapshots: snapshots).save(identity)
  let savedSecrets = secrets.data
  let fixture = InventoryFixture()
  let coordinator = EnrollmentCoordinator(
    service: InventoryEnrollment(), browser: TestBrowser(),
    secrets: secrets, snapshots: snapshots, inventoryService: fixture)
  async let first: Void = coordinator.refresh()
  async let second: Void = coordinator.refresh()
  _ = await (first, second)
  #expect(await fixture.calls == 1)
  #expect(coordinator.inventory?.revision == 2)
  #expect(coordinator.dnsInventory?.zone == "mesh.example")
  #expect(secrets.data == savedSecrets)  // No configuration ACK/persistence or identity mutation.
  await coordinator.foregroundContact()
  #expect(await fixture.calls == 1)  // Foreground ticks bounded to 30 seconds across windows.
  await fixture.update(revision: 1)
  await coordinator.refresh()
  #expect(coordinator.inventory?.revision == 2)
  #expect(coordinator.inventoryMessage != nil)
  await fixture.update(revision: 3, dnsRevision: 2)
  await coordinator.refresh()
  #expect(coordinator.inventory?.revision == 3)
  #expect(coordinator.dnsInventory == nil)
  #expect(coordinator.dnsMessage != nil)
  await fixture.update(error: .revoked)
  await coordinator.refresh()
  #expect(coordinator.snapshot?.phase == .revoked)
  #expect(coordinator.inventory == nil)
  #expect(coordinator.dnsInventory == nil)
}
