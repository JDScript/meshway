import Foundation
import MeshwayCore
import Security

public enum EnrollmentPhase: String, Codable, Sendable {
  case notSignedIn, awaitingBrowser, redeeming, enrolled, revoked
}

public struct EnrollmentSnapshot: Codable, Sendable, Equatable {
  public var server: String
  public var phase: EnrollmentPhase
  public var node: EnrolledNode?
}

// One Keychain item is the write-ahead journal. The binding is never assembled
// from independently written fields. Public metadata is mirrored separately.
struct IdentityJournal: Codable, Equatable, CustomStringConvertible, CustomDebugStringConvertible {
  var description: String { "IdentityJournal(<redacted>)" }
  var debugDescription: String { description }
  var credential: String
  var snapshot: EnrollmentSnapshot
  var pending: PendingBinding?
}
struct PendingBinding: Codable, Equatable, CustomStringConvertible, CustomDebugStringConvertible {
  var description: String { "PendingBinding(<redacted>)" }
  var debugDescription: String { description }
  var url: URL
  var verifier: String
  var state: String
  var name: String
  var version: String
  var code: String?
}

public enum PersistenceError: Error, LocalizedError {
  case keychain, corrupt, file
  public var errorDescription: String? {
    switch self {
    case .keychain:
      "Keychain is unavailable. No plaintext fallback is used. Your identity has not been replaced."
    case .corrupt:
      "Saved enrollment is inconsistent. Remove it from this device explicitly. Its identity has not been replaced."
    case .file: "Cannot persist enrollment state. Your Keychain identity has been preserved."
    }
  }
}

@MainActor public protocol SecretStore {
  func read() throws -> Data?
  func write(_ data: Data) throws
  func remove() throws
}

@MainActor public final class KeychainSecretStore: SecretStore {
  private let service: String
  private let accessGroup: String?
  public init(service: String? = nil, accessGroup: String? = nil) {
    self.service = service ?? (Bundle.main.bundleIdentifier ?? "dev.meshway") + ".device-identity"
    self.accessGroup = accessGroup
  }
  private var query: [String: Any] {
    var query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service, kSecAttrAccount as String: "identity-journal",
      kSecAttrSynchronizable as String: false,
    ]
    if let accessGroup, !accessGroup.isEmpty { query[kSecAttrAccessGroup as String] = accessGroup }
    return query
  }
  public func read() throws -> Data? {
    var query = query
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)
    if status == errSecItemNotFound { return nil }
    guard status == errSecSuccess, let data = result as? Data else {
      throw PersistenceError.keychain
    }
    return data
  }
  public func write(_ data: Data) throws {
    let attributes: [String: Any] = [
      kSecValueData as String: data,
      kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
    ]
    let status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
    if status == errSecItemNotFound {
      let insert = query.merging(attributes) { _, new in new }
      guard SecItemAdd(insert as CFDictionary, nil) == errSecSuccess else {
        throw PersistenceError.keychain
      }
    } else if status != errSecSuccess {
      throw PersistenceError.keychain
    }
  }
  public func remove() throws {
    let status = SecItemDelete(query as CFDictionary)
    guard status == errSecSuccess || status == errSecItemNotFound else {
      throw PersistenceError.keychain
    }
  }
}

@MainActor public protocol EnrollmentSnapshotStore {
  func read() throws -> EnrollmentSnapshot?
  func write(_ snapshot: EnrollmentSnapshot?) throws
}

@MainActor public final class FileEnrollmentSnapshotStore: EnrollmentSnapshotStore {
  private let url: URL
  public init(url: URL? = nil) {
    self.url =
      url
      ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
      .appendingPathComponent(Bundle.main.bundleIdentifier ?? "Meshway", isDirectory: true)
      .appendingPathComponent("enrollment.json")
  }
  public func read() throws -> EnrollmentSnapshot? {
    guard FileManager.default.fileExists(atPath: url.path) else { return nil }
    do {
      return try JSONDecoder().decode(EnrollmentSnapshot.self, from: Data(contentsOf: url))
    } catch { throw PersistenceError.corrupt }
  }
  public func write(_ snapshot: EnrollmentSnapshot?) throws {
    do {
      if let snapshot {
        try FileManager.default.createDirectory(
          at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try JSONEncoder().encode(snapshot).write(to: url, options: .atomic)
      } else if FileManager.default.fileExists(atPath: url.path) {
        try FileManager.default.removeItem(at: url)
      }
    } catch { throw PersistenceError.file }
  }
}

@MainActor final class IdentityPersistence {
  let secrets: any SecretStore
  let snapshots: any EnrollmentSnapshotStore
  init(secrets: any SecretStore, snapshots: any EnrollmentSnapshotStore) {
    self.secrets = secrets
    self.snapshots = snapshots
  }
  func load() throws -> IdentityJournal? {
    guard let data = try secrets.read() else {
      // Missing secrets with a retained public identity are not a fresh
      // device. Require explicit reset instead of silently swapping identity.
      guard try snapshots.read() == nil else { throw PersistenceError.corrupt }
      return nil
    }
    let journal: IdentityJournal
    do { journal = try JSONDecoder().decode(IdentityJournal.self, from: data) } catch {
      throw PersistenceError.corrupt
    }
    guard journal.credential.count == 64,
      journal.credential.allSatisfy({ "0123456789abcdef".contains($0) }),
      (journal.snapshot.phase == .awaitingBrowser || journal.snapshot.phase == .redeeming)
        == (journal.pending != nil),
      journal.snapshot.phase != .awaitingBrowser || journal.pending?.code == nil,
      journal.snapshot.phase != .redeeming || journal.pending?.code != nil,
      journal.snapshot.phase != .enrolled || journal.snapshot.node != nil,
      journal.snapshot.phase == .enrolled || journal.snapshot.phase == .revoked
        || journal.snapshot.node == nil
    else { throw PersistenceError.corrupt }
    // Repair mirror after a crash between the two ordered atomic writes.
    try snapshots.write(journal.snapshot)
    return journal
  }
  func save(_ journal: IdentityJournal) throws {
    do { try secrets.write(JSONEncoder().encode(journal)) } catch let error as PersistenceError {
      throw error
    } catch { throw PersistenceError.keychain }
    try snapshots.write(journal.snapshot)
  }
  func reset() throws {
    // Remove the mirror first. If removal of the Keychain item fails/crashes,
    // its journal repairs the mirror on restart and reset can be repeated.
    // Once the secret is removed there is no dangling public identity that
    // could be mistaken for accidental Keychain loss.
    try snapshots.write(nil)
    try secrets.remove()
  }
}
