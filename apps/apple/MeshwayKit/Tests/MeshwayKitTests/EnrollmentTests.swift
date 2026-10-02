import Foundation
import MeshwayCore
import Testing

@testable import MeshwayModel

@MainActor final class MemorySecrets: SecretStore {
  var data: Data?
  var failWrites = false
  var failRemoves = false
  func read() throws -> Data? { data }
  func write(_ data: Data) throws {
    if failWrites { throw PersistenceError.keychain }
    self.data = data
  }
  func remove() throws {
    if failRemoves { throw PersistenceError.keychain }
    data = nil
  }
}
@MainActor final class MemorySnapshots: EnrollmentSnapshotStore {
  var snapshot: EnrollmentSnapshot?
  var failWrites = false
  func read() throws -> EnrollmentSnapshot? { snapshot }
  func write(_ snapshot: EnrollmentSnapshot?) throws {
    if failWrites { throw PersistenceError.file }
    self.snapshot = snapshot
  }
}

private let state = "saved-state+中文"
private let code = String(repeating: "a", count: 64)
private let nodeCredential = String(repeating: "b", count: 64)
private let verifier = String(repeating: "c", count: 64)
private let server = "https://console.test"
private let node = EnrolledNode(
  id: "node-123", name: "Original Mac", meshIPv4: "100.64.0.1", meshIPv6: "fd00::1")

private func callback(_ query: [URLQueryItem]) -> URL {
  var parts = URLComponents(string: "meshway://enroll/callback")!
  parts.queryItems = query
  return parts.url!
}

@Test func strictCallbackValidation() throws {
  let items = [URLQueryItem(name: "state", value: state), URLQueryItem(name: "code", value: code)]
  #expect(try DeviceCallback.validate(callback(items), state: state) == .code(code))
  #expect(
    try DeviceCallback.validate(
      callback([items[0], URLQueryItem(name: "error", value: "access_denied")]), state: state)
      == .denied)
  for invalid in [
    callback(items + [items[0]]), callback(items + [items[1]]),
    callback(items + [URLQueryItem(name: "error", value: "access_denied")]),
    callback([items[1]]), callback([items[0]]),
    callback([items[0], URLQueryItem(name: "code", value: "")]),
    callback([items[0], URLQueryItem(name: "error", value: "other")]),
    URL(string: "meshway://evil/callback?state=x&code=y")!,
    URL(string: "meshway://enroll/callback/??state=x")!,
    URL(string: "meshway://enroll:1/callback?state=x")!,
    URL(string: "meshway://enroll/%63allback?state=x")!,
    URL(string: "meshway://enroll/callback?state=x#fragment")!,
    callback(items + [URLQueryItem(name: "extra", value: "x")]),
  ] { #expect(throws: BrowserError.self) { try DeviceCallback.validate(invalid, state: state) } }
  #expect(throws: BrowserError.self) {
    try DeviceCallback.validate(callback(items), state: "wrong")
  }
}

actor TestEnrollmentService: EnrollmentService {
  nonisolated func credential() throws -> String { nodeCredential }
  nonisolated func begin(server: String, name: String, version: String) throws
    -> PendingAuthorization
  {
    PendingAuthorization(
      url: URL(string: "https://console.test/authorize-device")!, verifier: verifier, state: state)
  }
  var requests: [String] = []
  var exchangeError: EnrollmentError?
  var heartbeatError: EnrollmentError?
  func setExchangeError(_ error: EnrollmentError?) { exchangeError = error }
  func setHeartbeatError(_ error: EnrollmentError?) { heartbeatError = error }
  func exchange(
    server: String, code: String, verifier: String, credential: String, name: String,
    version: String
  ) async throws -> EnrolledNode {
    requests.append([server, code, verifier, credential, name, version].joined(separator: "|"))
    if let exchangeError { throw exchangeError }
    return node
  }
  func heartbeat(server: String, credential: String) async throws -> EnrolledNode {
    if let heartbeatError { throw heartbeatError }
    return node
  }
}

@MainActor final class TestBrowser: BrowserAuthorizing {
  var failure: BrowserError? = .unavailable
  var opens = 0
  var beforeOpen: (() throws -> Void)?
  func authorize(_ url: URL) async throws -> URL {
    opens += 1
    try beforeOpen?()
    if let failure { throw failure }
    return callback([
      URLQueryItem(name: "state", value: state), URLQueryItem(name: "code", value: code),
    ])
  }
  func cancel() {}
}

@MainActor @Test func recoverEveryPersistedPhaseAndExactBinding() async throws {
  let secrets = MemorySecrets()
  let snapshots = MemorySnapshots()
  let browser = TestBrowser()
  let service = TestEnrollmentService()
  func restored() -> EnrollmentCoordinator {
    EnrollmentCoordinator(
      service: service, browser: browser, secrets: secrets, snapshots: snapshots)
  }
  let first = restored()
  browser.beforeOpen = {
    let saved = try #require(secrets.data)
    let journal = try JSONDecoder().decode(IdentityJournal.self, from: saved)
    #expect(journal.credential == nodeCredential)
    #expect(journal.snapshot.phase == .awaitingBrowser)
    #expect(journal.pending?.verifier == verifier)
  }
  await first.signIn(server: server, name: "Original Mac", version: "original-v1")
  #expect(first.snapshot?.phase == .awaitingBrowser)
  #expect(restored().snapshot?.phase == .awaitingBrowser)
  browser.beforeOpen = nil
  // Process dies after callback persistence, before any exchange.
  let afterBrowser = restored()
  try afterBrowser.receiveCallback(
    callback([URLQueryItem(name: "state", value: state), URLQueryItem(name: "code", value: code)]))
  #expect(await service.requests.isEmpty)
  #expect(afterBrowser.snapshot?.phase == .redeeming)
  await service.setExchangeError(.transport)
  let afterCallback = restored()
  await afterCallback.resume()
  #expect(afterCallback.snapshot?.phase == .redeeming)
  await service.setExchangeError(nil)
  let afterLostResponse = restored()
  await afterLostResponse.signIn(server: server, name: "Changed name", version: "changed-version")
  #expect(afterLostResponse.snapshot?.phase == .enrolled)
  let requests = await service.requests
  #expect(requests.count == 2)
  #expect(requests[0] == requests[1])
  #expect(
    requests[0]
      == [server, code, verifier, nodeCredential, "Original Mac", "original-v1"].joined(
        separator: "|"))
  let enrolled = restored()
  #expect(enrolled.snapshot?.node == node)
  #expect(
    try JSONDecoder().decode(IdentityJournal.self, from: #require(secrets.data)).pending == nil)
  await enrolled.heartbeat()
  #expect(enrolled.heartbeatTime != nil)
  await service.setHeartbeatError(.revoked)
  await enrolled.heartbeat()
  #expect(enrolled.snapshot?.phase == .revoked)
  let revoked = restored()
  #expect(revoked.snapshot?.phase == .revoked)
  await revoked.signIn(server: server, name: "Mac", version: "v2")
  #expect(await service.requests.count == 2)
  revoked.reset()
  #expect(secrets.data == nil)
  #expect(restored().snapshot == nil)
}

@MainActor @Test func cancelledAndDeniedPreserveCredentialAndKeychainFailureNeverOpensBrowser()
  async throws
{
  for failure in [BrowserError.cancelled, .denied] {
    let secrets = MemorySecrets()
    let snapshots = MemorySnapshots()
    let browser = TestBrowser()
    browser.failure = failure
    let coordinator = EnrollmentCoordinator(
      service: TestEnrollmentService(), browser: browser, secrets: secrets, snapshots: snapshots)
    await coordinator.signIn(server: server, name: "Mac", version: "v1")
    #expect(coordinator.snapshot?.phase == .awaitingBrowser)
    let journal = try JSONDecoder().decode(IdentityJournal.self, from: #require(secrets.data))
    #expect(journal.credential == nodeCredential)
    #expect(journal.pending?.state == state)
    let restored = EnrollmentCoordinator(
      service: TestEnrollmentService(), browser: browser, secrets: secrets, snapshots: snapshots)
    #expect(restored.snapshot?.phase == .awaitingBrowser)
  }
  let secrets = MemorySecrets()
  let snapshots = MemorySnapshots()
  let browser = TestBrowser()
  let coordinator = EnrollmentCoordinator(
    service: TestEnrollmentService(), browser: browser, secrets: secrets, snapshots: snapshots)
  secrets.failWrites = true
  await coordinator.signIn(server: server, name: "Mac", version: "v1")
  #expect(browser.opens == 0)
  #expect(coordinator.message != nil)
  #expect(coordinator.hasIdentity)  // recovery blocks silent regeneration
}

@MainActor @Test func interruptedMirrorWriteRepairsFromKeychainAndExpiredRestartRetainsIdentity()
  async throws
{
  let secrets = MemorySecrets()
  let snapshots = MemorySnapshots()
  let browser = TestBrowser()
  let service = TestEnrollmentService()
  let coordinator = EnrollmentCoordinator(
    service: service, browser: browser, secrets: secrets, snapshots: snapshots)
  snapshots.failWrites = true
  await coordinator.signIn(server: server, name: "Mac", version: "v1")
  #expect(browser.opens == 0)
  #expect(secrets.data != nil)
  snapshots.failWrites = false
  let restored = EnrollmentCoordinator(
    service: service, browser: browser, secrets: secrets, snapshots: snapshots)
  #expect(restored.snapshot?.phase == .awaitingBrowser)
  #expect(snapshots.snapshot == restored.snapshot)
  try restored.receiveCallback(
    callback([URLQueryItem(name: "state", value: state), URLQueryItem(name: "code", value: code)]))
  await service.setExchangeError(.invalidCode)
  await restored.resume()
  #expect(restored.canRestart)
  await restored.restartExpiredAttempt()
  #expect(restored.snapshot?.phase == .awaitingBrowser)
  let journal = try JSONDecoder().decode(IdentityJournal.self, from: #require(secrets.data))
  #expect(journal.credential == nodeCredential)
  #expect(journal.pending?.code == nil)
  #expect(journal.snapshot.node == nil)
}

@MainActor @Test func journalRepairsInterruptedPublicWritesAtEveryCommitBoundary() throws {
  for phase in [EnrollmentPhase.notSignedIn, .awaitingBrowser, .redeeming, .enrolled, .revoked] {
    let secrets = MemorySecrets()
    let snapshots = MemorySnapshots()
    let persistence = IdentityPersistence(secrets: secrets, snapshots: snapshots)
    let hasPending = phase == .awaitingBrowser || phase == .redeeming
    let journal = IdentityJournal(
      credential: nodeCredential,
      snapshot: EnrollmentSnapshot(
        server: server, phase: phase, node: phase == .enrolled || phase == .revoked ? node : nil),
      pending: hasPending
        ? PendingBinding(
          url: URL(string: "https://console.test/authorize-device")!,
          verifier: verifier, state: state, name: "Original Mac", version: "original-v1",
          code: phase == .redeeming ? code : nil) : nil)
    snapshots.failWrites = true
    #expect(throws: PersistenceError.self) { try persistence.save(journal) }
    #expect(secrets.data != nil)
    snapshots.failWrites = false
    let recovered = try #require(try persistence.load())
    #expect(recovered.snapshot == journal.snapshot)
    #expect(recovered.credential == nodeCredential)
    #expect(recovered.pending?.code == journal.pending?.code)
    #expect(recovered.pending?.verifier == journal.pending?.verifier)
    #expect(snapshots.snapshot == journal.snapshot)
  }
}

@MainActor @Test func publicFileContainsNoSecretsAndCorruptPrivateStateBlocksRegeneration() throws {
  let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
  defer { try? FileManager.default.removeItem(at: directory) }
  let url = directory.appendingPathComponent("enrollment.json")
  let store = FileEnrollmentSnapshotStore(url: url)
  let snapshot = EnrollmentSnapshot(server: server, phase: .redeeming, node: nil)
  try store.write(snapshot)
  #expect(try store.read() == snapshot)
  let text = try String(contentsOf: url, encoding: .utf8)
  for secret in [nodeCredential, code, verifier, state] { #expect(!text.contains(secret)) }
  let secrets = MemorySecrets()
  secrets.data = Data("corrupt".utf8)
  let coordinator = EnrollmentCoordinator(
    service: TestEnrollmentService(), browser: TestBrowser(), secrets: secrets, snapshots: store)
  #expect(coordinator.hasIdentity)
  #expect(coordinator.message != nil)
  #expect(secrets.data == Data("corrupt".utf8))
}

@MainActor @Test func missingSecretsNeverSwapIdentityAndInterruptedResetRecoversOriginal() throws {
  let secrets = MemorySecrets()
  let snapshots = MemorySnapshots()
  snapshots.snapshot = EnrollmentSnapshot(server: server, phase: .enrolled, node: node)
  let missing = EnrollmentCoordinator(
    service: TestEnrollmentService(), browser: TestBrowser(), secrets: secrets, snapshots: snapshots
  )
  #expect(missing.hasIdentity)
  #expect(missing.message != nil)
  #expect(snapshots.snapshot?.node == node)
  let persistence = IdentityPersistence(secrets: secrets, snapshots: snapshots)
  let journal = IdentityJournal(
    credential: nodeCredential, snapshot: try #require(snapshots.snapshot), pending: nil)
  try persistence.save(journal)
  secrets.failRemoves = true
  #expect(throws: PersistenceError.self) { try persistence.reset() }
  #expect(snapshots.snapshot == nil)
  let recovered = try #require(try persistence.load())
  #expect(recovered.credential == nodeCredential)
  #expect(recovered.snapshot.node == node)
  #expect(snapshots.snapshot?.node == node)
  secrets.failRemoves = false
  try persistence.reset()
  #expect(try persistence.load() == nil)
}
