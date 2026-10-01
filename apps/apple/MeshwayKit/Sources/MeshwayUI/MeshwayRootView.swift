import MeshwayModel
import SwiftUI

private enum Destination: String, CaseIterable, Identifiable {
  case home = "Home"
  case network = "Network"
  case settings = "Settings"
  var id: Self { self }
  var symbol: String {
    switch self {
    case .home: "house"
    case .network: "point.3.connected.trianglepath.dotted"
    case .settings: "gearshape"
    }
  }
}

public struct MeshwayRootView: View {
  @Bindable private var model: AppModel
  @State private var selection: Destination? = .home
  #if os(iOS)
    @Environment(\.horizontalSizeClass) private var sizeClass
  #endif
  public init(model: AppModel) { self.model = model }
  public var body: some View {
    #if os(iOS)
      if sizeClass == .compact {
        TabView {
          ForEach(Destination.allCases) { destination in
            NavigationStack { content(destination).navigationTitle(destination.rawValue) }
              .tabItem { Label(destination.rawValue, systemImage: destination.symbol) }
          }
        }
      } else {
        splitView
      }
    #else
      splitView.frame(minWidth: 720, minHeight: 480)
    #endif
  }
  private var splitView: some View {
    NavigationSplitView {
      List(Destination.allCases, selection: $selection) { destination in
        Label(destination.rawValue, systemImage: destination.symbol).tag(destination)
      }.navigationTitle("Meshway")
    } detail: {
      content(selection ?? .home).navigationTitle((selection ?? .home).rawValue)
    }
  }
  @ViewBuilder private func content(_ destination: Destination) -> some View {
    switch destination {
    case .home: home
    case .network:
      ContentUnavailableView(
        "No network data", systemImage: "network",
        description: Text(
          "Sign-in and the node inventory data source are not available yet. No peers have been loaded."
        ))
    case .settings: settings
    }
  }
  private var home: some View {
    Form {
      Section {
        Label(model.status.title, systemImage: model.status.symbol).font(.title2)
        Text(model.authUnavailableReason).foregroundStyle(.secondary)
      } header: {
        Text("Node status")
      }
      controllerSection
      Section("Sign in") {
        Button("Sign in with OAuth") {}.disabled(true)
        Text(model.authUnavailableReason).font(.callout).foregroundStyle(.secondary)
      }
      Section("Tunnel unavailable") {
        Text(model.tunnelUnavailableReason).foregroundStyle(.secondary)
      }
    }.formStyle(.grouped)
  }
  private var controllerSection: some View {
    Section("Controller") {
      TextField("https://controller.example.com", text: $model.controllerDraft)
        .autocorrectionDisabled()
        #if os(iOS)
          .textInputAutocapitalization(.never).keyboardType(.URL)
        #endif
      Button("Save controller") { model.saveController() }
      if let message = model.validationMessage {
        Text(message).foregroundStyle(.red)
      }
      if let address = model.controller {
        LabeledContent("Saved URL", value: address.url.absoluteString)
        Text("Saved locally; controller reachability has not been checked.").font(.caption)
        Button("Clear controller", role: .destructive) { model.clearController() }
      }
    }
  }
  private var settings: some View {
    Form {
      controllerSection
      Section("About Meshway") {
        LabeledContent("Rust core", value: model.coreInfo.version)
        LabeledContent("Configuration schema", value: String(model.coreInfo.configurationSchema))
        LabeledContent("Routing rule limit", value: String(model.coreInfo.maximumRoutingRules))
        Text(
          "This device will join as a Meshway node. Authentication and packet forwarding are not implemented in this build."
        )
      }
      Section("Diagnostics") {
        Text(model.preferencesDiagnostic)
        Button("Inspect system VPN preferences") {
          Task { await model.inspectPreferences() }
        }
        Text("Traffic statistics and diagnostics export are not available.").foregroundStyle(
          .secondary)
      }
    }.formStyle(.grouped)
  }
}

public struct MeshwayMenuView: View {
  private let model: AppModel
  public init(model: AppModel) { self.model = model }
  public var body: some View {
    Text(model.status.title)
    Text("Tunnel unavailable in this build")
    Divider()
    Text("Rust core \(model.coreInfo.version)")
  }
}
