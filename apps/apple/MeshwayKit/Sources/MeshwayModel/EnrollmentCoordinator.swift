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
  public private(set) var profiles: [SavedEnrollment] = []
  public private(set) var activeProfileID: String?
  public private(set) var switching = false
  public var hasIdentity: Bool { journal != nil || recoveryBlocked }
  public var canRestart: Bool {
    error == .invalidCode && journal?.snapshot.node == nil && journal?.pending?.code != nil
  }
  @ObservationIgnored private let service: any EnrollmentService
  @ObservationIgnored private let inventoryService: any InventoryService
  @ObservationIgnored private var lastForegroundRefresh: Date?
  @ObservationIgnored private let browser: any BrowserAuthorizing
  @ObservationIgnored private let persistence: IdentityPersistence
  @ObservationIgnored private let profileStore: EnrollmentProfiles?
  @ObservationIgnored private var journal: IdentityJournal?
  @ObservationIgnored private var recoveryBlocked = false
  @ObservationIgnored private var generation = 0
  @ObservationIgnored private var heartbeatBusy = false
  @ObservationIgnored private var drainWaiters: [CheckedContinuation<Void, Never>] = []

  public init(
    service: any EnrollmentService = RustEnrollmentService(),
    browser: any BrowserAuthorizing = SystemBrowserAuthorization(),
    secrets: any SecretStore = KeychainSecretStore(),
    snapshots: any EnrollmentSnapshotStore = FileEnrollmentSnapshotStore(),
    inventoryService: any InventoryService = RustEnrollmentService(),
    profiles: EnrollmentProfiles? = nil
  ) {
    self.service = service
    self.inventoryService = inventoryService
    self.browser = browser
    profileStore = profiles
    persistence = IdentityPersistence(secrets: secrets, snapshots: snapshots)
    do {
      if let profiles { journal = try profiles.load() } else { journal = try persistence.load() }
      snapshot = journal?.snapshot
      syncProfiles()
    } catch {
      recoveryBlocked = true
      syncProfiles()
      report(error)
    }
  }

  private func syncProfiles() {
    profiles = profileStore?.profiles ?? []
    activeProfileID = profileStore?.activeID
  }

  private struct Fence: Equatable {
    let profile: String?
    let generation: Int
  }
  private var fence: Fence { Fence(profile: activeProfileID, generation: generation) }

  /// Invalidate results immediately, then drain the old operation before loading
  /// a new identity. Cancellation alone cannot fence a completed HTTP response.
  private func drain() async {
    generation += 1
    browser.cancel()
    inventory = nil
    dnsInventory = nil
    if busy || heartbeatBusy || refreshing {
      await withCheckedContinuation { drainWaiters.append($0) }
    }
    clearDisplay()
  }

  private func resumeDrainIfIdle() {
    guard !busy, !heartbeatBusy, !refreshing else { return }
    let waiters = drainWaiters
    drainWaiters = []
    for waiter in waiters { waiter.resume() }
  }

  private func clearDisplay() {
    journal = nil
    snapshot = nil
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
  }

  public func switchProfile(_ id: String?) async {
    guard let profileStore, !switching, !recoveryBlocked || id != nil else { return }
    switching = true
    defer { switching = false }
    await drain()
    do {
      try profileStore.select(id)
      journal = try id.flatMap { try profileStore.journal($0) }
      snapshot = journal?.snapshot
      recoveryBlocked = false
      syncProfiles()
    } catch {
      recoveryBlocked = true
      report(error)
    }
  }

  /// Opens the sign-in screen. Identity is generated only on explicit sign-in.
  public func addNetwork() async { await switchProfile(nil) }

  public func removeActiveProfile() async {
    guard let profileStore, !switching, activeProfileID != nil || recoveryBlocked else { return }
    let id = activeProfileID
    switching = true
    defer { switching = false }
    await drain()
    do {
      if let id { try profileStore.remove(id) } else { try profileStore.discardUnmigratedLegacy() }
      journal = try profileStore.load()
      snapshot = journal?.snapshot
      recoveryBlocked = false
      syncProfiles()
    } catch {
      recoveryBlocked = true
      report(error)
      syncProfiles()
    }
  }

  private func report(_ failure: Error) {
    if let failure = failure as? EnrollmentError {
      error = failure
      message = failure.localizedDescription
    } else if let failure = failure as? BrowserError {
      message = failure.localizedDescription
    } else if let failure = failure as? PersistenceError {
      recoveryBlocked = true
      syncProfiles()
      message = failure.localizedDescription
    } else {
      message = "Enrollment failed. The saved identity and binding have been preserved."
    }
  }

  private func save(_ next: IdentityJournal) throws {
    do {
      if let profileStore, let activeProfileID {
        try profileStore.save(next, id: activeProfileID)
      } else {
        try persistence.save(next)
      }
      syncProfiles()
    } catch {
      // A mirror write can fail after Keychain committed. Reload before any
      // further operation; never retry using stale in-memory binding fields.
      recoveryBlocked = true
      throw error
    }
    journal = next
    snapshot = next.snapshot
  }

  public func signIn(server: String, name: String, version: String) async {
    guard !busy, !switching, !recoveryBlocked else { return }
    if let journal {
      guard journal.snapshot.server == server else {
        message = "Use Add network… to sign in to another server with a separate identity."
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
      if let profileStore, activeProfileID == nil {
        _ = try profileStore.create(server: server)
        syncProfiles()
      }
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
    guard !busy, !switching, !recoveryBlocked, let journal, let pending = journal.pending else {
      return
    }
    busy = true
    let currentGeneration = fence
    defer {
      busy = false
      resumeDrainIfIdle()
    }
    do {
      if pending.code == nil {
        let callback = try await browser.authorize(pending.url)
        guard fence == currentGeneration else { return }
        try receiveCallback(callback)
      }
      if self.journal?.snapshot.phase == .redeeming {
        try await redeem(generation: currentGeneration)
      }
    } catch {
      guard fence == currentGeneration else { return }
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
    guard !switching, var next = journal, let pending = next.pending,
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
    guard !busy, !switching, !recoveryBlocked else { return }
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

  private func redeem(generation: Fence) async throws {
    guard var next = journal, let pending = next.pending, let code = pending.code else {
      throw PersistenceError.corrupt
    }
    let node = try await service.exchange(
      server: next.snapshot.server, code: code,
      verifier: pending.verifier, credential: next.credential, name: pending.name,
      version: pending.version)
    guard fence == generation else { return }
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
    // Cancellation/denial retains the exact pending identity and browser binding.
    next.snapshot.phase = .awaitingBrowser
    try save(next)
  }

  public func restartExpiredAttempt() async {
    guard !busy, !switching, !recoveryBlocked, canRestart, var next = journal,
      let pending = next.pending
    else {
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
    guard !refreshing else { return }
    await contact()
  }

  private func contact() async {
    guard !busy, !switching, !heartbeatBusy, !recoveryBlocked, let journal,
      journal.snapshot.phase == .enrolled
    else { return }
    heartbeatBusy = true
    let currentGeneration = fence
    defer {
      heartbeatBusy = false
      resumeDrainIfIdle()
    }
    do {
      let node = try await service.heartbeat(
        server: journal.snapshot.server, credential: journal.credential)
      guard fence == currentGeneration else { return }
      guard node.id == journal.snapshot.node?.id else { throw EnrollmentError.server }
      heartbeatTime = Date()
      heartbeatSucceeded = true
      heartbeatResult = "Controller contact succeeded (not tunnel connectivity)."
    } catch {
      guard fence == currentGeneration else { return }
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
    guard !refreshing, !heartbeatBusy, !busy, !switching, !recoveryBlocked, let journal,
      journal.snapshot.phase == .enrolled, let node = journal.snapshot.node
    else { return }
    refreshing = true
    lastForegroundRefresh = Date()
    let currentGeneration = fence
    defer {
      refreshing = false
      resumeDrainIfIdle()
    }
    await contact()
    guard snapshot?.phase == .enrolled, fence == currentGeneration else { return }
    do {
      let next = try await inventoryService.configuration(
        server: journal.snapshot.server, credential: journal.credential, nodeID: node.id)
      guard fence == currentGeneration, snapshot?.phase == .enrolled else { return }
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
      if let profileStore, let activeProfileID {
        try profileStore.label(
          id: activeProfileID,
          zone: profiles.first { $0.id == activeProfileID }?.zone, prefix: next.ipv4Prefix)
        syncProfiles()
      }
      // DNS is optional. Never combine DNS from a different configuration revision.
      dnsInventory = nil
      do {
        let dns = try await inventoryService.dns(
          server: journal.snapshot.server, credential: journal.credential, nodeID: node.id)
        guard fence == currentGeneration, snapshot?.phase == .enrolled else { return }
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
        if let profileStore, let activeProfileID {
          try profileStore.label(id: activeProfileID, zone: dns.zone, prefix: next.ipv4Prefix)
          syncProfiles()
        }
      } catch {
        guard fence == currentGeneration else { return }
        dnsMessage =
          "DNS names could not be refreshed. "
          + ((error as? EnrollmentError)?.localizedDescription ?? "Please try again.")
        if error as? EnrollmentError == .revoked {
          report(error)
          markRevoked()
        }
      }
    } catch {
      guard fence == currentGeneration else { return }
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
    if profileStore != nil {
      Task { await removeActiveProfile() }
      return
    }
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
