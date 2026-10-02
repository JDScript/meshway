import Foundation

public struct SavedEnrollment: Codable, Identifiable, Equatable, Sendable {
  public let id: String
  public var snapshot: EnrollmentSnapshot?
  public var server: String?
  public var zone: String?
  public var ipv4Prefix: String?
  public var label: String {
    zone ?? ipv4Prefix ?? snapshot?.node?.meshIPv4 ?? "Sign-in · \(id.prefix(6))"
  }
  public var serverHost: String {
    (snapshot?.server ?? server).flatMap { URL(string: $0)?.host } ?? "Saved enrollment"
  }
  public var badge: String? {
    switch snapshot?.phase {
    case .enrolled: nil
    case .revoked: "Revoked"
    default: "Pending"
    }
  }
}

struct ProfileCatalog: Codable {
  var profiles: [SavedEnrollment] = []
  var removed: [String] = []
  var migrated = false
}

/// Public metadata only. Atomic files also serve as durable migration/removal intents.
@MainActor public final class ProfileMetadataStore: SecretStore {
  private let url: URL
  public init(url: URL) { self.url = url }
  public func read() throws -> Data? {
    guard FileManager.default.fileExists(atPath: url.path) else { return nil }
    do { return try Data(contentsOf: url) } catch { throw PersistenceError.file }
  }
  public func write(_ data: Data) throws {
    do {
      try FileManager.default.createDirectory(
        at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
      try data.write(to: url, options: .atomic)
    } catch { throw PersistenceError.file }
  }
  public func remove() throws {
    if FileManager.default.fileExists(atPath: url.path) {
      do { try FileManager.default.removeItem(at: url) } catch { throw PersistenceError.file }
    }
  }
}

@MainActor public final class EnrollmentProfiles {
  private let legacy: IdentityPersistence
  private let catalogStore: any SecretStore
  private let activeStore: any SecretStore
  private let identity: (String) -> IdentityPersistence
  private var catalog = ProfileCatalog()
  public private(set) var activeID: String?
  public var profiles: [SavedEnrollment] { catalog.profiles }

  init(
    legacy: IdentityPersistence, catalog: any SecretStore, active: any SecretStore,
    identity: @escaping (String) -> IdentityPersistence
  ) {
    self.legacy = legacy
    catalogStore = catalog
    activeStore = active
    self.identity = identity
  }

  public static func system(
    service: String? = nil, directory: URL? = nil,
    accessGroup: String? = nil
  ) -> EnrollmentProfiles {
    let service = service ?? (Bundle.main.bundleIdentifier ?? "dev.meshway") + ".device-identity"
    let directory =
      directory
      ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
      .appendingPathComponent(Bundle.main.bundleIdentifier ?? "Meshway", isDirectory: true)
    return EnrollmentProfiles(
      legacy: IdentityPersistence(
        secrets: KeychainSecretStore(service: service, accessGroup: accessGroup),
        snapshots: FileEnrollmentSnapshotStore(
          url: directory.appendingPathComponent("enrollment.json"))),
      catalog: ProfileMetadataStore(url: directory.appendingPathComponent("profiles.json")),
      active: ProfileMetadataStore(url: directory.appendingPathComponent("active-profile.json")),
      identity: { id in
        IdentityPersistence(
          secrets: KeychainSecretStore(
            service: service + ".profile." + id, accessGroup: accessGroup),
          snapshots: FileEnrollmentSnapshotStore(
            url: directory.appendingPathComponent("profiles/\(id).json")))
      })
  }

  func load() throws -> IdentityJournal? {
    if let data = try catalogStore.read() {
      do { catalog = try JSONDecoder().decode(ProfileCatalog.self, from: data) } catch {
        throw PersistenceError.corrupt
      }
    }
    guard Set(catalog.profiles.map(\.id)).count == catalog.profiles.count,
      (catalog.profiles.map(\.id) + catalog.removed).allSatisfy({
        $0 == "legacy" || UUID(uuidString: $0) != nil
      })
    else { throw PersistenceError.corrupt }
    if !catalog.migrated {
      // Deterministic destination, copy-before-delete. A crash never replaces an
      // already copied identity; the old journal remains until pointer commit.
      if let old = try legacy.load() {
        let destination = identity("legacy")
        if let copied = try destination.load() {
          guard copied == old else { throw PersistenceError.corrupt }
        } else {
          try destination.save(old)
        }
        if !catalog.profiles.contains(where: { $0.id == "legacy" }) {
          catalog.profiles.insert(SavedEnrollment(id: "legacy", snapshot: old.snapshot), at: 0)
          try saveCatalog()
        }
        if try activeStore.read() == nil { try select("legacy") }
        try legacy.reset()
      }
      catalog.migrated = true
      try saveCatalog()
    }
    try cleanRemoved()
    if let data = try activeStore.read() {
      do { activeID = try JSONDecoder().decode(String?.self, from: data) } catch {
        throw PersistenceError.corrupt
      }
    }
    for index in catalog.profiles.indices {
      guard let journal = try identity(catalog.profiles[index].id).load() else {
        // Interrupted creation remains visible, never silently regenerated.
        if catalog.profiles[index].snapshot != nil { throw PersistenceError.corrupt }
        continue
      }
      catalog.profiles[index].snapshot = journal.snapshot
    }
    try saveCatalog()
    if let activeID, !catalog.profiles.contains(where: { $0.id == activeID }) {
      try select(fallback())
    }
    return try activeID.flatMap { try identity($0).load() }
  }

  func create(server: String? = nil) throws -> String {
    let id = UUID().uuidString
    catalog.profiles.append(SavedEnrollment(id: id, snapshot: nil, server: server))
    try saveCatalog()  // Intent precedes identity: no unlisted secret if interrupted.
    try select(id)
    return id
  }
  func select(_ id: String?) throws {
    guard id == nil || catalog.profiles.contains(where: { $0.id == id }) else {
      throw PersistenceError.corrupt
    }
    try activeStore.write(JSONEncoder().encode(id))
    activeID = id
  }
  func journal(_ id: String) throws -> IdentityJournal? {
    guard let profile = catalog.profiles.first(where: { $0.id == id }) else {
      throw PersistenceError.corrupt
    }
    let journal = try identity(id).load()
    guard journal != nil || profile.snapshot == nil else { throw PersistenceError.corrupt }
    return journal
  }
  func save(_ journal: IdentityJournal, id: String) throws {
    guard let index = catalog.profiles.firstIndex(where: { $0.id == id }) else {
      throw PersistenceError.corrupt
    }
    try identity(id).save(journal)
    catalog.profiles[index].snapshot = journal.snapshot
    catalog.profiles[index].server = journal.snapshot.server
    try saveCatalog()
  }
  func label(id: String, zone: String?, prefix: String) throws {
    guard let index = catalog.profiles.firstIndex(where: { $0.id == id }) else { return }
    catalog.profiles[index].zone = zone
    catalog.profiles[index].ipv4Prefix = prefix
    try saveCatalog()
  }
  func remove(_ id: String) throws {
    catalog.profiles.removeAll { $0.id == id }
    if !catalog.removed.contains(id) { catalog.removed.append(id) }
    try saveCatalog()  // Tombstone precedes deleting profile data.
    if activeID == id { try select(fallback()) }
    try cleanRemoved()
  }
  func discardUnmigratedLegacy() throws {
    guard !catalog.migrated, catalog.profiles.isEmpty else { throw PersistenceError.corrupt }
    // Explicit local removal is the only escape from a corrupt legacy identity.
    try legacy.reset()
    catalog.migrated = true
    try saveCatalog()
  }
  private func fallback() -> String? {
    catalog.profiles.first { $0.snapshot?.phase == .enrolled }?.id
  }
  private func cleanRemoved() throws {
    for id in catalog.removed { try identity(id).reset() }
    catalog.removed = []
    try saveCatalog()
  }
  private func saveCatalog() throws { try catalogStore.write(JSONEncoder().encode(catalog)) }
}
