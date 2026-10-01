import Foundation
import MeshwayCore
import MeshwayModel
import Testing

@Test func rustBridgeUsesSharedLogic() {
  let core = RustCoreService()
  #expect(core.buildInfo().version == "0.1.0")
  #expect(core.buildInfo().configurationSchema == 1)
  #expect(core.isValidTagSelector("tag:production"))
  #expect(!core.isValidTagSelector("production"))
}

@Test func asyncFFIBridgeSanitizesTransportFailure() async {
  do {
    _ = try await RustEnrollmentService().heartbeat(
      server: "http://127.0.0.1:1", credential: String(repeating: "a", count: 64))
    Issue.record("Unexpected node response from a closed test port")
  } catch {
    #expect(error as? EnrollmentError == .transport)
    #expect(!error.localizedDescription.contains(String(repeating: "a", count: 64)))
  }
}

@Test func controllerInputValidation() throws {
  #expect(try ControllerAddress(" https://example.com:8443 ").url.host == "example.com")
  for value in [
    "example.com", "file:///tmp/a", "https://", "https://user:secret@example.com",
    "https://example.com?q=x", "https://example.com#x", "http://example.com:0",
    "https://example.com/controller", "http://remote.test",
  ] {
    #expect(throws: AddressError.self) { try ControllerAddress(value) }
  }
}

@MainActor @Test func configurationNeverClaimsSignInAndInvalidDraftKeepsSavedAddress() throws {
  let suite = "MeshwayTests.\(UUID().uuidString)"
  let defaults = try #require(UserDefaults(suiteName: suite))
  defer { defaults.removePersistentDomain(forName: suite) }
  let model = AppModel(
    enrollment: EnrollmentCoordinator(secrets: MemorySecrets(), snapshots: MemorySnapshots()),
    tunnel: SystemTunnelController(providerBundleID: "dev.example.meshway.PacketTunnel"),
    defaults: defaults)
  #expect(model.status == .notConfigured)
  model.controllerDraft = "https://example.com"
  model.saveController()
  #expect(model.status == .notSignedIn)
  model.controllerDraft = "invalid"
  model.saveController()
  #expect(model.validationMessage != nil)
  #expect(model.controller?.url.host == "example.com")
  let restored = AppModel(
    enrollment: EnrollmentCoordinator(secrets: MemorySecrets(), snapshots: MemorySnapshots()),
    tunnel: SystemTunnelController(providerBundleID: "dev.example.meshway.PacketTunnel"),
    defaults: defaults)
  #expect(restored.controller == model.controller)
  model.clearController()
  #expect(model.status == .notConfigured)
}
