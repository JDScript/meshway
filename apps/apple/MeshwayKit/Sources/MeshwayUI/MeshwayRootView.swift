import Foundation
import MeshwayCore
import MeshwayModel
import SwiftUI

public struct MeshwayRootView: View {
  @Bindable private var model: AppModel
  @State private var search = ""
  @State private var selected: MeshDevice?
  @State private var confirmReset = false
  @State private var showNetwork = false
  @Environment(\.scenePhase) private var scenePhase
  public init(model: AppModel) { self.model = model }

  public var body: some View {
    NavigationStack {
      Group {
        if model.enrollment.snapshot?.phase == .enrolled {
          devices
        } else {
          OnboardingView(model: model)
        }
      }
      .navigationTitle("Meshway")
      #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        .toolbarTitleMenu { networkActions }
      #endif
      .toolbar {
        #if os(macOS)
          ToolbarItem(placement: .navigation) { networkMenu }
        #else
          ToolbarItem(placement: .principal) { networkMenu }
        #endif
        ToolbarItem(placement: .primaryAction) { ConnectSwitch() }
      }
    }
    #if os(macOS)
      .frame(minWidth: 440, idealWidth: 500, maxWidth: 680, minHeight: 520, idealHeight: 680)
    #endif
    .tint(.indigo)
    #if DEBUG && os(macOS)
      .onReceive(
        NotificationCenter.default.publisher(for: Notification.Name("MeshwaySnapshotAction"))
      ) {
        guard ProcessInfo.processInfo.environment["MESHWAY_SNAPSHOT_DIR"] != nil,
          let action = $0.object as? String
        else { return }
        switch action {
        case "detail": selected = state.peers.last
        case "network": showNetwork = true
        case "dismiss":
          selected = nil
          showNetwork = false
        case "search": search = "no matching device"
        default: break
        }
      }
    #endif
    .onOpenURL { url in Task { await model.enrollment.handleIncomingCallback(url) } }
    .task(id: model.enrollment.snapshot?.phase) {
      if model.enrollment.snapshot?.phase == .enrolled && scenePhase == .active {
        await model.enrollment.foregroundContact()
      }
    }
    .task(id: scenePhase) {
      guard scenePhase == .active else { return }
      while !Task.isCancelled {
        await model.enrollment.foregroundContact()
        do { try await Task.sleep(for: .seconds(30)) } catch { return }
      }
    }
    .sheet(item: $selected) { device in
      DeviceDetailView(
        device: device, dnsName: state.dnsName(device), isSelf: device.id == state.ownDevice?.id)
    }
    .sheet(isPresented: $showNetwork) { NetworkDetailView(model: model) }
    .confirmationDialog("Reset this device?", isPresented: $confirmReset, titleVisibility: .visible)
    {
      Button("Reset local identity", role: .destructive) { model.enrollment.reset() }
    } message: {
      Text(
        "This removes the local credential and pending sign-in. It does not revoke this node on the server. Revoke it separately in the console."
      )
    }
  }

  private var state: DeviceScreenState { DeviceScreenState(model: model) }
  private var networkMenu: some View {
    Menu {
      networkActions
    } label: {
      NetworkLabel(title: state.networkTitle)
    }.menuStyle(.borderlessButton).accessibilityLabel("Select network: \(state.networkTitle)")
  }
  @ViewBuilder private var networkActions: some View {
    if let server = model.enrollment.snapshot?.server ?? model.controller?.url.absoluteString {
      if model.enrollment.snapshot?.phase == .enrolled {
        Label(state.networkTitle, systemImage: "checkmark")
      } else {
        Text("Server: \(state.networkTitle)")
      }
      Text(server)
      Divider()
      Button("Network details", systemImage: "info.circle") { showNetwork = true }
      Button("Copy server URL", systemImage: "doc.on.doc") { Clipboard.copy(server) }
    } else {
      Text("No network selected")
    }
    if model.enrollment.hasIdentity {
      Divider()
      Button(
        "Reset this device…", systemImage: "rectangle.portrait.and.arrow.right", role: .destructive
      ) {
        confirmReset = true
      }.disabled(model.enrollment.busy || model.enrollment.refreshing)
    }
  }
  @ViewBuilder private var devices: some View {
    #if os(iOS)
      List {
        Section { TunnelNotice() }
        if let message = state.error {
          Section { RefreshError(message: message, hasInventory: state.loaded) }
        }
        if let own = state.ownDevice {
          Section("This device") {
            deviceButton(own, own: true)
            HeartbeatCaption(result: state.heartbeatResult, time: state.heartbeatTime)
          }
        }
        Section("Devices · \(state.peers.count)") {
          if state.filtered(search).isEmpty { DeviceEmptyState(state: state, search: search) }
          ForEach(state.filtered(search)) { device in deviceButton(device, own: false) }
        }
        Section { InventoryFooter(state: state) }
      }
      .listStyle(.insetGrouped)
      .searchable(text: $search, prompt: "Name, IP or DNS name")
      .refreshable { await model.enrollment.refresh() }
    #else
      VStack(spacing: 0) {
        HStack(spacing: 12) {
          SearchBar(text: $search)
          Button("Refresh devices", systemImage: "arrow.clockwise") {
            Task { await model.enrollment.refresh() }
          }
          .labelStyle(.iconOnly).buttonStyle(.borderless).disabled(state.loading)
          .help("Refresh devices and contact controller")
        }.padding(.horizontal, 24).padding(.top, 18).padding(.bottom, 14)
        DeviceContentView(state: state, search: search, select: { selected = $0 })
      }
      .background(Surface.canvas)
    #endif
  }
  private func deviceButton(_ device: MeshDevice, own: Bool) -> some View {
    Button {
      selected = device
    } label: {
      DeviceRow(device: device, dnsName: state.dnsName(device), isSelf: own)
    }
    .buttonStyle(.plain)
    .contextMenu { CopyDeviceActions(device: device, dnsName: state.dnsName(device)) }
    #if os(iOS)
      .swipeActions(edge: .trailing) {
        Button("Copy IPv4", systemImage: "doc.on.doc") { Clipboard.copy(device.meshIPv4) }.tint(
          .indigo)
      }
    #endif
  }
}

public struct MeshwayMenuView: View {
  private let model: AppModel
  @Environment(\.openWindow) private var openWindow
  @Environment(\.scenePhase) private var scenePhase
  @State private var confirmReset = false
  @State private var selected: MeshDevice?
  public init(model: AppModel) { self.model = model }
  public var body: some View {
    VStack(spacing: 0) {
      MeshwayPanelHeader(title: DeviceScreenState(model: model).networkTitle) {
        if let server = model.enrollment.snapshot?.server {
          if model.enrollment.snapshot?.phase == .enrolled {
            Label(DeviceScreenState(model: model).networkTitle, systemImage: "checkmark")
          }
          Text(server)
          Button("Copy server URL") { Clipboard.copy(server) }
          Divider()
          Button("Reset this device…", role: .destructive) { confirmReset = true }
            .disabled(model.enrollment.refreshing || model.enrollment.busy)
        } else {
          Text("No network selected")
        }
      }
      Divider()
      if model.enrollment.snapshot?.phase == .enrolled {
        DeviceContentView(
          state: DeviceScreenState(model: model), compact: true, select: { selected = $0 }
        )
        .frame(height: 460)
      } else {
        VStack(spacing: 12) {
          MeshwayGlyph(size: 44)
          Text(model.status.title).font(.headline)
          if let message = model.enrollment.message {
            Text(message).font(.caption).foregroundStyle(.secondary)
          }
          Text("Open Meshway to sign in with your browser.").font(.callout).foregroundStyle(
            .secondary)
        }.padding(24)
      }
      Divider()
      HStack {
        Button("Open Meshway") { openWindow(id: "main") }
        Spacer()
        #if os(macOS)
          Button("Quit") { NSApplication.shared.terminate(nil) }
        #endif
      }.buttonStyle(.borderless).padding(16)
    }
    .frame(width: 360).background(Surface.canvas).tint(.indigo)
    .sheet(item: $selected) { device in
      DeviceDetailView(
        device: device, dnsName: DeviceScreenState(model: model).dnsName(device),
        isSelf: device.id == model.enrollment.snapshot?.node?.id)
    }
    .task(id: scenePhase) {
      guard scenePhase == .active else { return }
      while !Task.isCancelled {
        await model.enrollment.foregroundContact()
        do { try await Task.sleep(for: .seconds(30)) } catch { return }
      }
    }
    .confirmationDialog("Reset this device?", isPresented: $confirmReset) {
      Button("Reset local identity", role: .destructive) { model.enrollment.reset() }
    } message: {
      Text("This only removes the local identity. It does not revoke the server-side node.")
    }
  }
}
