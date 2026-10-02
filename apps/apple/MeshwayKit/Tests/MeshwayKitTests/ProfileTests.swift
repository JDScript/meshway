import Foundation
import MeshwayCore
import Testing

@testable import MeshwayModel

@MainActor private final class Interruption {
  var remaining: Int?
  func step() throws {
    if let remaining {
      self.remaining = remaining - 1
      if remaining == 1 { throw PersistenceError.file }
    }
  }
}
@MainActor private final class DurableData: SecretStore {
  let interruption: Interruption
  var data: Data?
  init(_ interruption: Interruption) { self.interruption = interruption }
  func read() throws -> Data? { data }
  func write(_ data: Data) throws {
    try interruption.step()
    self.data = data
  }
  func remove() throws {
    try interruption.step()
    data = nil
  }
}
@MainActor private final class DurableSnapshot: EnrollmentSnapshotStore {
  let interruption: Interruption
  var snapshot: EnrollmentSnapshot?
  init(_ interruption: Interruption) { self.interruption = interruption }
  func read() throws -> EnrollmentSnapshot? { snapshot }
  func write(_ snapshot: EnrollmentSnapshot?) throws {
    try interruption.step()
    self.snapshot = snapshot
  }
}
@MainActor private final class ProfileFixture {
  let interruption = Interruption()
  let legacySecrets: DurableData
  let legacySnapshot: DurableSnapshot
  let catalog: DurableData
  let active: DurableData
  var identities: [String: (DurableData, DurableSnapshot)] = [:]
  init() {
    legacySecrets = DurableData(interruption)
    legacySnapshot = DurableSnapshot(interruption)
    catalog = DurableData(interruption)
    active = DurableData(interruption)
  }
  func persistence(_ id: String) -> IdentityPersistence {
    if identities[id] == nil {
      identities[id] = (DurableData(interruption), DurableSnapshot(interruption))
    }
    let stores = identities[id]!
    return IdentityPersistence(secrets: stores.0, snapshots: stores.1)
  }
  func repository() -> EnrollmentProfiles {
    EnrollmentProfiles(
      legacy: IdentityPersistence(secrets: legacySecrets, snapshots: legacySnapshot),
      catalog: catalog, active: active, identity: { self.persistence($0) })
  }
}

private func savedJournal(_ character: Character, phase: EnrollmentPhase = .enrolled)
  -> IdentityJournal
{
  let value = String(character)
  return IdentityJournal(
    credential: String(repeating: value, count: 64),
    snapshot: EnrollmentSnapshot(
      server: "https://console.test", phase: phase,
      node: phase == .enrolled || phase == .revoked
        ? EnrolledNode(id: value, name: value, meshIPv4: "100.64.0.1", meshIPv6: "fd00::1") : nil),
    pending: phase == .awaitingBrowser || phase == .redeeming
      ? PendingBinding(
        url: URL(string: "https://console.test/authorize-device")!,
        verifier: "original-verifier", state: "original-state", name: "original-name",
        version: "v1",
        code: phase == .redeeming ? "unredeemed-code" : nil) : nil)
}

@MainActor @Test func profileMigrationRecoversEveryInterruptedWriteAndDeletion() throws {
  for phase in [EnrollmentPhase.notSignedIn, .awaitingBrowser, .redeeming, .enrolled, .revoked] {
    for boundary in 1...16 {
      let fixture = ProfileFixture()
      let original = savedJournal("a", phase: phase)
      fixture.legacySecrets.data = try JSONEncoder().encode(original)
      fixture.legacySnapshot.snapshot = original.snapshot
      fixture.interruption.remaining = boundary
      _ = try? fixture.repository().load()
      fixture.interruption.remaining = nil
      let recovered = fixture.repository()
      let journal = try #require(try recovered.load())
      #expect(recovered.profiles.count == 1)
      #expect(recovered.activeID == "legacy")
      #expect(journal.credential == original.credential)
      #expect(journal.snapshot == original.snapshot)
      #expect(journal.pending?.code == original.pending?.code)
      #expect(journal.pending?.verifier == original.pending?.verifier)
      #expect(journal.pending?.state == original.pending?.state)
      #expect(fixture.legacySecrets.data == nil)
      #expect(try recovered.load()?.snapshot == original.snapshot)
    }
  }
}

@MainActor @Test func profilesIsolateCredentialsPendingRevocationAndRemovalFallback() throws {
  let fixture = ProfileFixture()
  let repository = fixture.repository()
  _ = try repository.load()
  let first = try repository.create()
  try repository.save(savedJournal("a"), id: first)
  let second = try repository.create()
  try repository.save(savedJournal("b", phase: .redeeming), id: second)
  let third = try repository.create()
  try repository.save(savedJournal("c", phase: .revoked), id: third)
  for (id, credential) in [(first, "a"), (second, "b"), (third, "c")] {
    try repository.select(id)
    #expect(try fixture.repository().load()?.credential == String(repeating: credential, count: 64))
  }
  #expect(try repository.journal(second)?.pending?.code == "unredeemed-code")
  #expect(try repository.journal(first)?.pending == nil)
  #expect(repository.profiles.last?.badge == "Revoked")
  try repository.remove(third)
  #expect(repository.activeID == first)
  try repository.remove(first)
  #expect(repository.activeID == nil)  // Pending enrollment is not auto-activated.
  try repository.select(second)
  try repository.remove(second)
  #expect(repository.profiles.isEmpty)
  #expect(try fixture.repository().load() == nil)
}

@MainActor @Test func profileRemovalRecoversEveryInterruptedBoundary() throws {
  for boundary in 1...10 {
    let fixture = ProfileFixture()
    let repository = fixture.repository()
    _ = try repository.load()
    let first = try repository.create()
    try repository.save(savedJournal("a"), id: first)
    let second = try repository.create()
    try repository.save(savedJournal("b"), id: second)
    fixture.interruption.remaining = boundary
    try? repository.remove(second)
    fixture.interruption.remaining = nil
    let restored = fixture.repository()
    _ = try restored.load()
    if restored.profiles.contains(where: { $0.id == second }) {
      try restored.remove(second)  // Crash before removal intent committed.
    }
    #expect(restored.activeID == first)
    #expect(try restored.journal(first)?.credential == String(repeating: "a", count: 64))
    #expect(try fixture.persistence(second).load() == nil)
  }
}

@MainActor @Test func interruptedCreationIsVisibleAndExplicitlyDiscardable() throws {
  let fixture = ProfileFixture()
  let repository = fixture.repository()
  _ = try repository.load()
  _ = try repository.create()  // Crash before the first journal write.
  let restored = fixture.repository()
  #expect(try restored.load() == nil)
  let pending = try #require(restored.profiles.first)
  #expect(pending.badge == "Pending")
  try restored.remove(pending.id)
  #expect(restored.profiles.isEmpty)
}

@MainActor @Test func migrationNeverDeletesConflictingDestinationAndMissingSecretsNeverRegenerate()
  throws
{
  let fixture = ProfileFixture()
  let original = savedJournal("a")
  fixture.legacySecrets.data = try JSONEncoder().encode(original)
  fixture.legacySnapshot.snapshot = original.snapshot
  try fixture.persistence("legacy").save(savedJournal("b"))
  #expect(throws: PersistenceError.self) { try fixture.repository().load() }
  #expect(fixture.legacySecrets.data != nil)
  #expect(try fixture.persistence("legacy").load()?.credential == String(repeating: "b", count: 64))

  let clean = ProfileFixture()
  let repository = clean.repository()
  _ = try repository.load()
  let id = try repository.create()
  try repository.save(original, id: id)
  clean.identities[id]?.0.data = nil
  clean.identities[id]?.1.snapshot = nil
  #expect(throws: PersistenceError.self) { try repository.journal(id) }
  #expect(throws: PersistenceError.self) { try clean.repository().load() }
  #expect(repository.profiles.first?.snapshot?.node == original.snapshot.node)
}

private actor HeldHeartbeat: EnrollmentService {
  var continuation: CheckedContinuation<EnrolledNode, Error>?
  var calls: [String] = []
  nonisolated func credential() throws -> String { String(repeating: "c", count: 64) }
  nonisolated func begin(server: String, name: String, version: String) throws
    -> PendingAuthorization
  {
    throw EnrollmentError.server
  }
  func exchange(
    server: String, code: String, verifier: String, credential: String, name: String,
    version: String
  ) async throws -> EnrolledNode {
    throw EnrollmentError.server
  }
  func heartbeat(server: String, credential: String) async throws -> EnrolledNode {
    calls.append(credential)
    return try await withCheckedThrowingContinuation { continuation = $0 }
  }
  func release(_ node: EnrolledNode) {
    continuation?.resume(returning: node)
    continuation = nil
  }
}

@MainActor @Test func switchingDrainsOldContactAndFencesLateResponse() async throws {
  let fixture = ProfileFixture()
  let repository = fixture.repository()
  _ = try repository.load()
  let first = try repository.create()
  try repository.save(savedJournal("a"), id: first)
  let second = try repository.create()
  try repository.save(savedJournal("b"), id: second)
  try repository.select(first)
  let service = HeldHeartbeat()
  let coordinator = EnrollmentCoordinator(
    service: service, browser: TestBrowser(), profiles: repository)
  let contact = Task { await coordinator.heartbeat() }
  while await service.calls.isEmpty { await Task.yield() }
  let switching = Task { await coordinator.switchProfile(second) }
  while !coordinator.switching { await Task.yield() }
  #expect(coordinator.inventory == nil)
  await coordinator.heartbeat()  // No request starts on the next identity yet.
  #expect(await service.calls.count == 1)
  await service.release(try #require(savedJournal("a").snapshot.node))
  await contact.value
  await switching.value
  #expect(coordinator.activeProfileID == second)
  #expect(coordinator.snapshot?.node?.id == "b")
  #expect(coordinator.heartbeatTime == nil)
  #expect(coordinator.heartbeatResult == nil)
  #expect(coordinator.inventory == nil)
  let nextContact = Task { await coordinator.heartbeat() }
  while await service.calls.count < 2 { await Task.yield() }
  #expect(
    await service.calls == [String(repeating: "a", count: 64), String(repeating: "b", count: 64)])
  await service.release(try #require(savedJournal("b").snapshot.node))
  await nextContact.value
  #expect(coordinator.heartbeatSucceeded == true)
}

private actor ImmediateContact: EnrollmentService {
  nonisolated func credential() throws -> String { try RustEnrollmentService().credential() }
  nonisolated func begin(server: String, name: String, version: String) throws
    -> PendingAuthorization
  {
    PendingAuthorization(
      url: URL(string: "https://console.test/authorize-device")!,
      verifier: String(repeating: "c", count: 64), state: "saved-state+中文")
  }
  var exchangedCredentials: [String] = []
  func exchange(
    server: String, code: String, verifier: String, credential: String, name: String,
    version: String
  ) async throws -> EnrolledNode {
    exchangedCredentials.append(credential)
    return EnrolledNode(id: credential, name: name, meshIPv4: "100.64.0.1", meshIPv6: "fd00::1")
  }
  func heartbeat(server: String, credential: String) async throws -> EnrolledNode {
    let character = try #require(credential.first)
    return try #require(savedJournal(character).snapshot.node)
  }
}

private actor HeldInventory: InventoryService {
  let holdDNS: Bool
  var waiting = false
  var continuation: CheckedContinuation<Void, Never>?
  init(holdDNS: Bool) { self.holdDNS = holdDNS }
  private func hold() async {
    waiting = true
    await withCheckedContinuation { continuation = $0 }
  }
  func release() {
    continuation?.resume()
    continuation = nil
  }
  func configuration(server: String, credential: String, nodeID: String) async throws
    -> MeshInventory
  {
    if !holdDNS { await hold() }
    return MeshInventory(
      revision: 1, digest: "old", ipv4Prefix: "100.64.0.0/24", ipv6Prefix: "fd00::/64",
      ownDevice: MeshDevice(
        id: nodeID, name: nodeID, country: nil, meshIPv4: "100.64.0.1", meshIPv6: "fd00::1"),
      peers: [])
  }
  func dns(server: String, credential: String, nodeID: String) async throws -> MeshDnsInventory {
    if holdDNS { await hold() }
    return MeshDnsInventory(revision: 1, digest: "old", zone: "old.example", names: [])
  }
}

@MainActor @Test func lateConfigurationAndDNSNeverCrossProfileBoundary() async throws {
  for holdDNS in [false, true] {
    let fixture = ProfileFixture()
    let repository = fixture.repository()
    _ = try repository.load()
    let first = try repository.create()
    try repository.save(savedJournal("a"), id: first)
    let second = try repository.create()
    try repository.save(savedJournal("b"), id: second)
    try repository.select(first)
    let inventory = HeldInventory(holdDNS: holdDNS)
    let coordinator = EnrollmentCoordinator(
      service: ImmediateContact(), browser: TestBrowser(),
      inventoryService: inventory, profiles: repository)
    let refresh = Task { await coordinator.refresh() }
    while await !inventory.waiting { await Task.yield() }
    let switching = Task { await coordinator.switchProfile(second) }
    while !coordinator.switching { await Task.yield() }
    #expect(coordinator.inventory == nil)
    await inventory.release()
    await refresh.value
    await switching.value
    #expect(coordinator.inventory == nil)
    #expect(coordinator.dnsInventory == nil)
    #expect(coordinator.inventoryTime == nil)
    #expect(coordinator.activeProfileID == second)
    #expect(coordinator.profiles.last?.zone == nil)
  }
}

@MainActor @Test func addingSameServerCreatesFreshIdentityAndCancellationResumesOriginal()
  async throws
{
  let fixture = ProfileFixture()
  let repository = fixture.repository()
  let browser = TestBrowser()
  let service = ImmediateContact()
  let coordinator = EnrollmentCoordinator(service: service, browser: browser, profiles: repository)
  browser.failure = .cancelled
  await coordinator.signIn(server: "https://console.test", name: "Mac", version: "v1")
  let pendingID = try #require(coordinator.activeProfileID)
  let original = try #require(try repository.journal(pendingID))
  #expect(coordinator.profiles.count == 1)
  #expect(coordinator.profiles.first?.badge == "Pending")
  browser.failure = nil
  await coordinator.resume()
  #expect(try repository.journal(pendingID)?.credential == original.credential)
  await coordinator.addNetwork()
  await coordinator.signIn(server: "https://console.test", name: "Mac", version: "v1")
  #expect(coordinator.profiles.count == 2)
  let secondID = try #require(coordinator.activeProfileID)
  let second = try #require(try repository.journal(secondID))
  #expect(pendingID != secondID)
  #expect(original.credential != second.credential)
  #expect(await service.exchangedCredentials == [original.credential, second.credential])
  #expect(try repository.journal(pendingID)?.snapshot.node?.id == original.credential)
  #expect(second.snapshot.node?.id == second.credential)
}

private actor HeldExchange: EnrollmentService {
  var continuation: CheckedContinuation<EnrolledNode, Never>?
  var waiting = false
  nonisolated func credential() throws -> String { String(repeating: "c", count: 64) }
  nonisolated func begin(server: String, name: String, version: String) throws
    -> PendingAuthorization
  {
    PendingAuthorization(
      url: URL(string: "https://console.test/authorize-device")!,
      verifier: String(repeating: "c", count: 64), state: "saved-state+中文")
  }
  func exchange(
    server: String, code: String, verifier: String, credential: String, name: String,
    version: String
  ) async throws -> EnrolledNode {
    waiting = true
    return await withCheckedContinuation { continuation = $0 }
  }
  func heartbeat(server: String, credential: String) async throws -> EnrolledNode {
    throw EnrollmentError.server
  }
  func release(_ node: EnrolledNode) {
    continuation?.resume(returning: node)
    continuation = nil
  }
}

@MainActor @Test func switchingDuringExchangeKeepsOldRecoveryBindingAndNeverCommitsIntoNewProfile()
  async throws
{
  let fixture = ProfileFixture()
  let repository = fixture.repository()
  _ = try repository.load()
  let enrolled = try repository.create()
  try repository.save(savedJournal("a"), id: enrolled)
  try repository.select(nil)
  let service = HeldExchange()
  let browser = TestBrowser()
  browser.failure = nil
  let coordinator = EnrollmentCoordinator(service: service, browser: browser, profiles: repository)
  let signIn = Task {
    await coordinator.signIn(server: "https://console.test", name: "Mac", version: "v1")
  }
  while await !service.waiting { await Task.yield() }
  let pendingID = try #require(coordinator.activeProfileID)
  let before = try #require(try repository.journal(pendingID))
  let switching = Task { await coordinator.switchProfile(enrolled) }
  while !coordinator.switching { await Task.yield() }
  await service.release(try #require(savedJournal("c").snapshot.node))
  await signIn.value
  await switching.value
  let after = try #require(try repository.journal(pendingID))
  #expect(after.snapshot.phase == .redeeming)
  #expect(after.credential == before.credential)
  #expect(after.pending?.code == before.pending?.code)
  #expect(after.pending?.verifier == before.pending?.verifier)
  #expect(coordinator.activeProfileID == enrolled)
  #expect(coordinator.snapshot?.node?.id == "a")
  #expect(try repository.journal(enrolled)?.credential == String(repeating: "a", count: 64))
}
