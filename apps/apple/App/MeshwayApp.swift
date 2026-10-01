import MeshwayModel
import MeshwayUI
import SwiftUI

@main struct MeshwayApp: App {
  @State private var model = AppModel(
    enrollment: EnrollmentCoordinator(
      secrets: KeychainSecretStore(
        accessGroup: Bundle.main.object(forInfoDictionaryKey: "MeshwayKeychainAccessGroup")
          as? String)),
    tunnel: SystemTunnelController(
      providerBundleID: (Bundle.main.bundleIdentifier ?? "dev.example.meshway") + ".PacketTunnel"))

  var body: some Scene {
    WindowGroup { MeshwayRootView(model: model) }
    #if os(macOS)
      MenuBarExtra("Meshway", systemImage: "network") {
        MeshwayMenuView(model: model)
        Button("Quit Meshway") { NSApplication.shared.terminate(nil) }
      }
    #endif
  }
}
