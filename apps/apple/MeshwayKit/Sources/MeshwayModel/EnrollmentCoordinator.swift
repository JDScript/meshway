import Foundation
import MeshwayCore
import Observation

@MainActor @Observable public final class EnrollmentCoordinator {
  public private(set) var snapshot: EnrollmentSnapshot?
  public private(set) var message: String?
  public private(set) var error: EnrollmentError?
  public private(set) var busy = false
  public private(set) var heartbeatTime: Date?
  public private(set) var heartbeatResult: String?
  public private(set) var heartbeatSucceeded: Bool?
  public private(set) var inventory: MeshInventory?
  public private(set) var dnsInventory: MeshDnsInventory?
  public private(set) var inventoryMessage: String?
  public private(set) var dnsMessage: String?
  public private(set) var refreshing = false
  public private(set) var inventoryTime: Date?
  public var hasIdentity: Bool { journal != nil || recoveryBlocked }
  public var canRestart: Bool {
    error == .invalidCode && journal?.snapshot.node == nil && journal?.pending?.code != nil
  }
  @ObservationIgnored private let service: any EnrollmentService
  @ObservationIgnored private let inventoryService: any InventoryService
  @ObservationIgnored private var lastForegroundRefresh: Date?
  @ObservationIgnored private let browser: any BrowserAuthorizing
  @ObservationIgnored private let persistence: IdentityPersistence
  @ObservationIgnored private var journal: IdentityJournal?
  @ObservationIgnored private var recoveryBlocked = false
  @ObservationIgnored private var generation = 0
  @ObservationIgnored private var heartbeatBusy = false

  public init(
    service: any EnrollmentService = RustEnrollmentService(),
    browser: any BrowserAuthorizing = SystemBrowserAuthorization(),
    secrets: any SecretStore = KeychainSecretStore(),
    snapshots: any EnrollmentSnapshotStore = FileEnrollmentSnapshotStore(),
    inventoryService: any InventoryService = RustEnrollmentService()
  ) {
    self.service = service
    self.inventoryService = inventoryService
    self.browser = browser
    persistence = IdentityPersistence(secrets: secrets, snapshots: snapshots)
    do {
      journal = try persistence.load()
      snapshot = journal?.snapshot
    } catch {
      recoveryBlocked = true
      report(error)
    }
  }

  private func report(_ failure: Error) {
    if let failure = failure as? EnrollmentError {
      error = failure
      message = failure.localizedDescription
    } else if let failure = failure as? BrowserError {
      message = failure.localizedDescription
    } else if let failure = failure as? PersistenceError {
      message = failure.localizedDescription
    } else {
      message = "Enrollment failed. The saved identity and binding have been preserved."
    }
  }

  private func save(_ next: IdentityJournal) throws {
    do { try persistence.save(next) } catch {
      // A mirror write can fail after Keychain committed. Reload before any
      // further operation; never retry using stale in-memory binding fields.
      recoveryBlocked = true
      throw error
    }
    journal = next
    snapshot = next.snapshot
  }

  public func signIn(server: String, name: String, version: String) async {
    guard !busy, !recoveryBlocked else { return }
    if let journal {
      guard journal.snapshot.server == server else {
        message = "Reset this device explicitly before changing its server."
        return
      }
      if journal.pending != nil {
        await resume()
        return
      }
      guard journal.snapshot.phase == .notSignedIn else { return }
    }
    do {
      let attempt = try service.begin(server: server, name: name, version: version)
      let credential = try journal?.credential ?? service.credential()
      let next = IdentityJournal(
        credential: credential,
        snapshot: EnrollmentSnapshot(server: server, phase: .awaitingBrowser, node: nil),
        pending: PendingBinding(
          url: attempt.url, verifier: attempt.verifier, state: attempt.state,
          name: name, version: version, code: nil))
      // The credential and complete attempt are committed before opening UI.
      try save(next)
      message = nil
      error = nil
      await resume()
    } catch { report(error) }
  }

  public func resume() async {
    guard !busy, !recoveryBlocked, let journal, let pending = journal.pending else { return }
    busy = true
    let currentGeneration = generation
    defer { if generation == currentGeneration { busy = false } }
    do {
      if pending.code == nil {
        let callback = try await browser.authorize(pending.url)
        guard generation == currentGeneration else { return }
        try receiveCallback(callback)
      }
      if self.journal?.snapshot.phase == .redeeming {
        try await redeem(generation: currentGeneration)
      }
    } catch {
      guard generation == currentGeneration else { return }
      if let failure = error as? BrowserError, failure == .cancelled || failure == .denied {
        do { try abandonBrowserAttempt() } catch {
          report(error)
          return
        }
      }
      report(error)
      if error as? EnrollmentError == .revoked { markRevoked() }
    }
  }

  public func receiveCallback(_ callback: URL) throws {
    guard var next = journal, let pending = next.pending,
      next.snapshot.phase == .awaitingBrowser, pending.code == nil
    else { throw BrowserError.invalidCallback }
    switch try DeviceCallback.validate(callback, state: pending.state) {
    case .denied: throw BrowserError.denied
    case .code(let code):
      next.pending?.code = code
      next.snapshot.phase = .redeeming
      // Never send the request before the received code is safely committed.
      try save(next)
    }
  }

  public func handleIncomingCallback(_ callback: URL) async {
    guard !busy, !recoveryBlocked else { return }
    do {
      try receiveCallback(callback)
      await resume()
    } catch {
      if error as? BrowserError == .denied {
        do { try abandonBrowserAttempt() } catch {
          report(error)
          return
        }
      }
      report(error)
    }
  }

  private func redeem(generation: Int) async throws {
    guard var next = journal, let pending = next.pending, let code = pending.code else {
      throw PersistenceError.corrupt
    }
    let node = try await service.exchange(
      server: next.snapshot.server, code: code,
      verifier: pending.verifier, credential: next.credential, name: pending.name,
      version: pending.version)
    guard self.generation == generation else { return }
    next.snapshot.node = node
    next.snapshot.phase = .enrolled
    next.pending = nil
    // Node plus clearing pending secrets is one Keychain transaction, then an
    // atomic public mirror. A lost response reuses the original entire binding.
    try save(next)
    message = nil
    error = nil
  }

  private func abandonBrowserAttempt() throws {
    guard var next = journal, next.pending?.code == nil else { throw PersistenceError.corrupt }
    next.pending = nil
    next.snapshot.phase = .notSignedIn
    try save(next)
  }

  public func restartExpiredAttempt() async {
    guard !busy, !recoveryBlocked, canRestart, var next = journal, let pending = next.pending else {
      return
    }
    do {
      // 401 is definitive for this exact saved binding. A consumed active
      // binding recovers even after expiry; 403/409 never permit this restart.
      let attempt = try service.begin(
        server: next.snapshot.server, name: pending.name, version: pending.version)
      next.pending = PendingBinding(
        url: attempt.url, verifier: attempt.verifier, state: attempt.state,
        name: pending.name, version: pending.version, code: nil)
      next.snapshot.phase = .awaitingBrowser
      try save(next)
      error = nil
      message = nil
      await resume()
    } catch { report(error) }
  }

  private func markRevoked() {
    inventory = nil
    dnsInventory = nil
    guard var next = journal else { return }
    next.snapshot.phase = .revoked
    next.pending = nil
    do { try save(next) } catch { report(error) }
  }

  public func heartbeat() async {
    guard !busy, !heartbeatBusy, !recoveryBlocked, let journal, journal.snapshot.phase == .enrolled
    else { return }
    heartbeatBusy = true
    let currentGeneration = generation
    defer { heartbeatBusy = false }
    do {
      let node = try await service.heartbeat(
        server: journal.snapshot.server, credential: journal.credential)
      guard generation == currentGeneration else { return }
      guard node.id == journal.snapshot.node?.id else { throw EnrollmentError.server }
      heartbeatTime = Date()
      heartbeatSucceeded = true
      heartbeatResult = "Controller contact succeeded (not tunnel connectivity)."
    } catch {
      guard generation == currentGeneration else { return }
      heartbeatTime = Date()
      heartbeatSucceeded = false
      heartbeatResult =
        (error as? EnrollmentError)?.localizedDescription ?? "Controller contact failed."
      if error as? EnrollmentError == .revoked {
        report(error)
        markRevoked()
      }
    }
  }

  public func foregroundContact() async {
    if snapshot?.phase == .redeeming { await resume() }
    guard lastForegroundRefresh.map({ Date().timeIntervalSince($0) >= 30 }) ?? true else { return }
    await refresh()
  }

  public func refresh() async {
    guard !refreshing, !busy, !recoveryBlocked, let journal,
      journal.snapshot.phase == .enrolled, let node = journal.snapshot.node
    else { return }
    refreshing = true
    lastForegroundRefresh = Date()
    let currentGeneration = generation
    defer { refreshing = false }
    await heartbeat()
    guard snapshot?.phase == .enrolled, generation == currentGeneration else { return }
    do {
      let next = try await inventoryService.configuration(
        server: journal.snapshot.server, credential: journal.credential, nodeID: node.id)
      guard generation == currentGeneration, snapshot?.phase == .enrolled else { return }
      guard next.ownDevice.id == node.id, next.ownDevice.meshIPv4 == node.meshIPv4,
        next.ownDevice.meshIPv6 == node.meshIPv6
      else { throw EnrollmentError.server }
      if let previous = inventory {
        guard next.revision >= previous.revision,
          next.revision != previous.revision || next.digest == previous.digest,
          next.ipv4Prefix == previous.ipv4Prefix, next.ipv6Prefix == previous.ipv6Prefix
        else { throw EnrollmentError.server }
      }
      inventory = next
      inventoryTime = Date()
      inventoryMessage = nil
      // DNS is optional. Never combine DNS from a different configuration revision.
      dnsInventory = nil
      do {
        let dns = try await inventoryService.dns(
          server: journal.snapshot.server, credential: journal.credential, nodeID: node.id)
        guard generation == currentGeneration, snapshot?.phase == .enrolled else { return }
        guard dns.revision == next.revision,
          dns.names.allSatisfy({ record in
            ([next.ownDevice] + next.peers).contains {
              $0.id == record.nodeID && $0.meshIPv4 == record.meshIPv4
                && $0.meshIPv6 == record.meshIPv6
            }
          })
        else { throw EnrollmentError.server }
        dnsInventory = dns
        dnsMessage = nil
      } catch {
        guard generation == currentGeneration else { return }
        dnsMessage =
          "DNS names could not be refreshed. "
          + ((error as? EnrollmentError)?.localizedDescription ?? "Please try again.")
        if error as? EnrollmentError == .revoked {
          report(error)
          markRevoked()
        }
      }
    } catch {
      guard generation == currentGeneration else { return }
      inventoryMessage =
        (error as? EnrollmentError)?.localizedDescription ?? "Devices could not be refreshed."
      if error as? EnrollmentError == .revoked {
        report(error)
        markRevoked()
      }
    }
  }

  /// Requires an explicit confirmation in UI. Does not revoke on the server.
  public func reset() {
    guard !busy, !heartbeatBusy, !refreshing else { return }
    do {
      try persistence.reset()
      generation += 1
      browser.cancel()
      journal = nil
      snapshot = nil
      recoveryBlocked = false
      message = nil
      error = nil
      heartbeatTime = nil
      heartbeatResult = nil
      heartbeatSucceeded = nil
      inventory = nil
      dnsInventory = nil
      inventoryMessage = nil
      dnsMessage = nil
      inventoryTime = nil
      lastForegroundRefresh = nil
    } catch {
      recoveryBlocked = true
      report(error)
    }
  }
}
