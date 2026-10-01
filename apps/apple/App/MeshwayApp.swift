import MeshwayModel
import MeshwayUI
import SwiftUI

@main struct MeshwayApp: App {
  @State private var model = Self.makeModel()

  @MainActor private static func makeModel() -> AppModel {
    #if DEBUG && os(macOS)
      if let model = WindowSnapshots.makeModel() { return model }
    #endif
    return AppModel(
      enrollment: EnrollmentCoordinator(
        secrets: KeychainSecretStore(
          accessGroup: Bundle.main.object(forInfoDictionaryKey: "MeshwayKeychainAccessGroup")
            as? String)),
      tunnel: SystemTunnelController(
        providerBundleID: (Bundle.main.bundleIdentifier ?? "dev.example.meshway") + ".PacketTunnel")
    )
  }

  var body: some Scene {
    WindowGroup(id: "main") {
      MeshwayRootView(model: model)
        #if DEBUG && os(macOS)
          .task { WindowSnapshots.start(model: model) }
        #endif
    }
    .defaultSize(width: 500, height: 680)
    #if os(macOS)
      MenuBarExtra("Meshway", systemImage: "network") {
        MeshwayMenuView(model: model)
      }.menuBarExtraStyle(.window)
    #endif
  }
}
