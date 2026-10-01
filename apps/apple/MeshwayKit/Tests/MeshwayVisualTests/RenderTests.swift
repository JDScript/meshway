// Preview-only fixtures. Never linked into the application targets.
import Foundation
import MeshwayCore
import MeshwayModel
import MeshwayUI
import SwiftUI
import Testing

#if os(macOS)
  import AppKit

  @Test @MainActor func renderAppleScreens() throws {
    guard let directory = ProcessInfo.processInfo.environment["MESHWAY_RENDER_DIR"] else { return }
    let output = URL(fileURLWithPath: directory, isDirectory: true)
    try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
    let own = MeshDevice(
      id: "preview-self", name: "Zeyu’s MacBook Pro", country: nil,
      meshIPv4: "100.64.0.4", meshIPv6: "fd42:6d65:7368::4")
    let peers = [
      ("Atlas", "US", "100.64.0.1", "fd42:6d65:7368::1"),
      ("Home studio", "JP", "100.64.0.2", "fd42:6d65:7368::2"),
      ("iPhone", "", "100.64.0.3", "fd42:6d65:7368::3"),
      ("Office workstation", "DE", "100.64.0.5", "fd42:6d65:7368::5"),
      ("Travel companion with a longer device name", "", "100.64.0.6", "fd42:6d65:7368::6"),
    ].enumerated().map { index, value in
      MeshDevice(
        id: "preview-\(index)", name: value.0, country: value.1.isEmpty ? nil : value.1,
        meshIPv4: value.2, meshIPv6: value.3)
    }
    let names = ([own] + peers).map {
      MeshDnsName(
        nodeID: $0.id,
        name: $0.id == own.id
          ? "macbook.mesh.example"
          : "\($0.name.lowercased().replacingOccurrences(of: " ", with: "-")).mesh.example",
        meshIPv4: $0.meshIPv4, meshIPv6: $0.meshIPv6)
    }
    let now = Date(timeIntervalSince1970: 1_790_839_200)
    let state = DeviceScreenState(
      networkTitle: "mesh.example", ownDevice: own, peers: peers, names: names,
      heartbeatResult: "Controller contact succeeded (not tunnel connectivity).",
      heartbeatTime: now, refreshedAt: now)
    for scheme in [ColorScheme.light, .dark] {
      let suffix = scheme == .light ? "light" : "dark"
      try render(
        shell(title: "Meshway") {
          onboarding(name: "Zeyu’s MacBook Pro")
        }, name: "mac-signed-out-\(suffix)", width: 500, height: 680, scheme: scheme, output: output
      )
      try render(
        shell(title: state.networkTitle) {
          searchChrome
          DeviceContentView(state: state).content
        }, name: "mac-devices-\(suffix)", width: 500, height: 1000, scheme: scheme, output: output)
      var failure = state
      failure.error = "Cannot contact the server. Your saved identity has been preserved."
      try render(
        shell(title: state.networkTitle) {
          searchChrome
          DeviceContentView(state: failure).content
        }, name: "mac-error-\(suffix)", width: 500, height: 1100, scheme: scheme, output: output)
      try render(
        shell(title: state.networkTitle) {
          DeviceContentView(state: state, compact: true).content
          Divider()
          HStack {
            Text("Open Meshway")
            Spacer()
            Text("Quit")
          }.font(.callout).padding(16)
        }, name: "menu-panel-\(suffix)", width: 360, height: 590, scheme: scheme, output: output)
      try render(
        shell(title: "Meshway") {
          onboarding(name: "Zeyu’s iPhone")
        }, name: "iphone-onboarding-\(suffix)", width: 393, height: 852, scheme: scheme,
        output: output)
      try render(
        shell(title: state.networkTitle) {
          searchChrome
          DeviceContentView(state: state).content
        }, name: "iphone-devices-\(suffix)", width: 393, height: 1000, scheme: scheme,
        output: output)
      try render(
        shell(title: "Meshway") {
          onboarding(name: own.name, revoked: true)
        }, name: "mac-revoked-\(suffix)", width: 500, height: 680, scheme: scheme, output: output)
      var loading = state
      loading.peers = []
      loading.loaded = false
      loading.loading = true
      loading.heartbeatResult = nil
      loading.heartbeatTime = nil
      loading.refreshedAt = nil
      loading.names = []
      try render(
        shell(title: state.networkTitle) { DeviceContentView(state: loading).content },
        name: "mac-loading-\(suffix)", width: 500, height: 680, scheme: scheme, output: output)
    }
  }

  @MainActor private func shell<Content: View>(title: String, @ViewBuilder content: () -> Content)
    -> some View
  {
    VStack(spacing: 0) {
      // Native Menu/Toggle/TextField/ScrollView are unsupported by ImageRenderer.
      // Use the real shared label and test-only native-chrome representations.
      HStack {
        NetworkLabel(title: title)
        Spacer()
        HStack(spacing: 8) {
          Text("Connect").foregroundStyle(.secondary)
          Capsule().fill(Color.secondary.opacity(0.12)).frame(width: 38, height: 22)
            .overlay(alignment: .leading) {
              Circle().fill(.secondary.opacity(0.3)).frame(width: 18, height: 18).padding(2)
            }
        }
      }.padding(18)
      Divider()
      content()
    }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
      .background(Color(nsColor: .windowBackgroundColor))
  }

  @MainActor private var searchChrome: some View {
    HStack(spacing: 8) {
      Image(systemName: "magnifyingglass")
      Text("Name, IP or DNS name")
      Spacer()
      Image(systemName: "arrow.clockwise").foregroundStyle(.indigo)
    }.foregroundStyle(.secondary).padding(10)
      .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 10))
      .overlay { RoundedRectangle(cornerRadius: 10).strokeBorder(.primary.opacity(0.07)) }
      .padding(.horizontal, 24).padding(.top, 18).padding(.bottom, 14)
  }

  @MainActor private func onboarding(name: String, revoked: Bool = false) -> some View {
    OnboardingLayout(
      status: revoked ? .revoked : .notSignedIn,
      message: revoked
        ? "Reset this device from the network menu to sign in again. This does not revoke the server-side node."
        : nil
    ) {
      Text(verbatim: "https://console.example.com").foregroundStyle(
        .secondary)
    } nameField: {
      Text(name)
    } action: {
      if !revoked { SignInButtonLabel().opacity(0.55) }
    }.frame(maxWidth: .infinity, maxHeight: .infinity)
  }

  @MainActor private func render<V: View>(
    _ view: V, name: String, width: CGFloat, height: CGFloat,
    scheme: ColorScheme, output: URL
  ) throws {
    NSApplication.shared.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
    let renderer = ImageRenderer(
      content: view.frame(width: width, height: height).environment(\.colorScheme, scheme).tint(
        .indigo))
    renderer.scale = 2
    let image = try #require(renderer.nsImage)
    let tiff = try #require(image.tiffRepresentation)
    let bitmap = try #require(NSBitmapImageRep(data: tiff))
    let png = try #require(bitmap.representation(using: .png, properties: [:]))
    try png.write(to: output.appendingPathComponent("\(name).png"))
  }
#endif
