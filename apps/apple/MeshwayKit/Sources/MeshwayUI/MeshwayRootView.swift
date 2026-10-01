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
  @State private var confirmReset = false
  @Environment(\.scenePhase) private var scenePhase
  #if os(iOS)
    @Environment(\.horizontalSizeClass) private var sizeClass
  #endif
  public init(model: AppModel) { self.model = model }
  public var body: some View {
    layout
      .onOpenURL { url in Task { await model.enrollment.handleIncomingCallback(url) } }
      .task(id: scenePhase) {
        guard scenePhase == .active else { return }
        while !Task.isCancelled {
          await model.enrollment.foregroundContact()
          do { try await Task.sleep(for: .seconds(30)) } catch { return }
        }
      }
      .confirmationDialog(
        "Reset this device?", isPresented: $confirmReset, titleVisibility: .visible
      ) {
        Button("Reset local identity", role: .destructive) { model.enrollment.reset() }
      } message: {
        Text(
          "This removes this device's local credential and pending sign-in. It does not revoke the node on the server. Use the console to revoke it separately."
        )
      }
  }
  @ViewBuilder private var layout: some View {
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
          "The peer inventory data source is not implemented. No peers have been loaded."
        ))
    case .settings: settings
    }
  }
  private var home: some View {
    Form {
      Section {
        Label(model.status.title, systemImage: model.status.symbol).font(.title2)
        if let message = model.enrollment.message { Text(message).foregroundStyle(.red) }
        if let node = model.enrollment.snapshot?.node {
          LabeledContent("Device", value: node.name)
          LabeledContent("Node ID", value: String(node.id.prefix(12)))
          LabeledContent("Mesh IPv4", value: node.meshIPv4)
          LabeledContent("Mesh IPv6", value: node.meshIPv6)
        }
        if let result = model.enrollment.heartbeatResult {
          Text(result).foregroundStyle(.secondary)
        }
        if let time = model.enrollment.heartbeatTime {
          LabeledContent("Last heartbeat") { Text(time, format: .dateTime) }
        }
      } header: {
        Text("Node status")
      }
      controllerSection
      Section("Sign in") {
        TextField("Device name", text: $model.deviceName).disabled(model.enrollment.hasIdentity)
        if [.notConfigured, .notSignedIn, .awaitingBrowser, .redeeming, .error].contains(
          model.status)
        {
          Button(
            model.enrollment.snapshot?.phase == .redeeming
              ? "Retry saved exchange" : "Sign in with browser"
          ) {
            Task { await model.signIn() }
          }.disabled(model.controller == nil || model.enrollment.busy)
        }
        if model.enrollment.canRestart {
          Button("Restart expired sign-in (keep identity)") {
            Task { await model.enrollment.restartExpiredAttempt() }
          }.disabled(model.enrollment.busy)
        }
        if model.enrollment.snapshot?.phase == .enrolled {
          Button("Contact controller now") { Task { await model.enrollment.heartbeat() } }
        }
        if model.enrollment.hasIdentity {
          Button("Reset this device", role: .destructive) { confirmReset = true }
            .disabled(model.enrollment.busy)
        }
        Text(
          "Your server is the console origin. Approval happens in the system browser; this app does not receive an administrator session."
        )
        .font(.callout).foregroundStyle(.secondary)
      }
      Section("Tunnel unavailable") {
        Text(model.tunnelUnavailableReason).foregroundStyle(.secondary)
      }
    }.formStyle(.grouped)
  }
  private var controllerSection: some View {
    Section("Server (console URL)") {
      TextField("https://controller.example.com", text: $model.controllerDraft)
        .disabled(model.enrollment.hasIdentity)
        .autocorrectionDisabled()
        #if os(iOS)
          .textInputAutocapitalization(.never).keyboardType(.URL)
        #endif
      Button("Save server") { model.saveController() }.disabled(model.enrollment.hasIdentity)
      if let message = model.validationMessage {
        Text(message).foregroundStyle(.red)
      }
      if let address = model.controller {
        LabeledContent("Saved URL", value: address.url.absoluteString)
        Text(
          "Saving a URL does not check reachability. Heartbeat reports control-plane contact only."
        ).font(.caption)
        Button("Clear server", role: .destructive) { model.clearController() }.disabled(
          model.enrollment.hasIdentity)
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
          "This device joins as a Meshway node. The tunnel and packet forwarding are not implemented in this build."
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
