#if os(macOS)
  import Foundation
  import MeshwayCore
  import MeshwayModel
  import Testing

  @MainActor private final class FixtureApprovalBrowser: BrowserAuthorizing {
    let environment: [String: String]
    init(environment: [String: String]) { self.environment = environment }
    func operation(_ mode: String, value: String) throws -> String {
      let process = Process()
      process.executableURL = URL(
        fileURLWithPath: try #require(environment["MESHWAY_APP_TEST_NODE"]))
      process.arguments = [
        try #require(environment["MESHWAY_APP_TEST_HELPER"]), mode,
        try #require(environment["MESHWAY_APP_TEST_CONTROLLER"]),
        try #require(environment["MESHWAY_APP_TEST_ADMIN_FILE"]), value,
      ]
      let output = Pipe()
      let errors = Pipe()
      process.standardOutput = output
      process.standardError = errors
      try process.run()
      let data = output.fileHandleForReading.readDataToEndOfFile()
      process.waitUntilExit()
      // Do not surface captured child output: it can include an exchange code.
      guard process.terminationStatus == 0, let text = String(data: data, encoding: .utf8) else {
        throw BrowserError.unavailable
      }
      return text
    }
    func authorize(_ url: URL) async throws -> URL {
      guard let callback = URL(string: try operation("authorize", value: url.absoluteString)) else {
        throw BrowserError.invalidCallback
      }
      return callback
    }
    func cancel() {}
  }

  @MainActor
  @Test(.enabled(if: ProcessInfo.processInfo.environment["MESHWAY_APP_TEST_CONSOLE"] != nil))
  func liveSwiftKeychainFFIProxyEnrollmentRecoveryAndRevocation() async throws {
    let environment = ProcessInfo.processInfo.environment
    let server = try #require(environment["MESHWAY_APP_TEST_CONSOLE"])
    let browser = FixtureApprovalBrowser(environment: environment)
    let secrets = KeychainSecretStore(service: "dev.meshway.signin-test.\(UUID().uuidString)")
    defer { try? secrets.remove() }
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      "meshway-signin-\(UUID().uuidString)")
    defer { try? FileManager.default.removeItem(at: directory) }
    let snapshots = FileEnrollmentSnapshotStore(
      url: directory.appendingPathComponent("enrollment.json"))
    let coordinator = EnrollmentCoordinator(
      browser: browser, secrets: secrets, snapshots: snapshots)
    await coordinator.signIn(server: server, name: "Swift Keychain smoke", version: "0.1.0")
    #expect(coordinator.message == nil)
    #expect(coordinator.snapshot?.phase == .enrolled)
    let node = try #require(coordinator.snapshot?.node)
    #expect(!node.meshIPv4.isEmpty && !node.meshIPv6.isEmpty)
    let restored = EnrollmentCoordinator(browser: browser, secrets: secrets, snapshots: snapshots)
    #expect(restored.snapshot?.node?.id == node.id)
    await restored.heartbeat()
    #expect(restored.heartbeatTime != nil)
    #expect(restored.heartbeatResult?.contains("succeeded") == true)
    let peerSecrets = KeychainSecretStore(service: "dev.meshway.signin-test.\(UUID().uuidString)")
    defer { try? peerSecrets.remove() }
    let peer = EnrollmentCoordinator(
      browser: browser, secrets: peerSecrets,
      snapshots: FileEnrollmentSnapshotStore(url: directory.appendingPathComponent("peer.json")))
    await peer.signIn(server: server, name: "Swift inventory peer", version: "0.1.0")
    #expect(peer.snapshot?.phase == .enrolled)
    await restored.refresh()
    #expect(restored.inventoryMessage == nil)
    #expect(restored.inventory?.ownDevice.id == node.id)
    #expect(restored.inventory?.peers.map(\.name) == ["Swift inventory peer"])
    #expect(restored.dnsMessage == nil)
    #expect(restored.dnsInventory?.revision == restored.inventory?.revision)
    _ = try browser.operation("revoke", value: node.id)
    await restored.heartbeat()
    #expect(restored.snapshot?.phase == .revoked)
    #expect(restored.inventory == nil)
    restored.reset()
    #expect(try secrets.read() == nil)
    #expect(try snapshots.read() == nil)
  }
#endif
