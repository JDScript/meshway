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
    Group {
      #if os(macOS)
        VStack(spacing: 0) {
          MeshwayPanelHeader(
            title: state.networkTitle == state.serverHost ? "Meshway" : state.networkTitle,
            subtitle: state.serverHost, compact: false,
            refreshing: state.loading,
            refresh: model.enrollment.snapshot?.phase == .enrolled
              ? { Task { await model.enrollment.refresh() } } : nil
          ) { networkActions }
          mainContent
        }.background(Surface.canvas)
      #else
        NavigationStack {
          mainContent
            .navigationTitle("Meshway")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
              ToolbarItem(placement: .principal) { NetworkLabel(title: state.networkTitle) }
              ToolbarItem(placement: .primaryAction) {
                Menu {
                  networkActions
                } label: {
                  Image(systemName: "ellipsis.circle")
                }
                .accessibilityLabel("Network actions")
              }
              ToolbarItem(placement: .primaryAction) { ConnectSwitch() }
            }
        }
      #endif
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
        case "alias-detail":
          selected = state.peers.first { state.dnsName($0) != state.canonicalDnsName($0) }
        case "network": showNetwork = true
        case "dismiss":
          selected = nil
          showNetwork = false
        case "search": search = search.isEmpty ? "no matching device" : ""
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
        device: device, dnsName: state.dnsName(device),
        canonicalName: state.canonicalDnsName(device),
        isSelf: device.id == state.ownDevice?.id)
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
  @ViewBuilder private var mainContent: some View {
    if model.enrollment.snapshot?.phase == .enrolled {
      devices
    } else {
      OnboardingView(model: model)
    }
  }
  @ViewBuilder private var networkActions: some View {
    if let server = model.enrollment.snapshot?.server ?? model.controller?.url.absoluteString {
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
        if let message = state.error, state.heartbeatSucceeded != false {
          Section { RefreshError(message: message, hasInventory: state.loaded) }
        }
        if let own = state.ownDevice {
          Section("This device") {
            deviceButton(own, own: true)
          }
        }
        Section {
          if state.filtered(search).isEmpty {
            DeviceEmptyState(state: state, search: search).frame(maxWidth: .infinity)
              .padding(.vertical, LayoutMetrics.panelInset)
          }
          ForEach(state.filtered(search)) { device in deviceButton(device, own: false) }
        } header: {
          HStack(spacing: LayoutMetrics.groupGap) {
            Text("Devices")
            Text("\(state.peers.count)")
            Spacer()
            InventoryUpdated(state: state)
          }
        }
        if let error = state.dnsError { Section { Text(error).foregroundStyle(.orange) } }
      }
      .listStyle(.insetGrouped)
      .searchable(text: $search, prompt: "Name, IP or DNS name")
      .refreshable { await model.enrollment.refresh() }
    #else
      VStack(spacing: 0) {
        SearchBar(text: $search).padding(.horizontal, LayoutMetrics.inset)
        DeviceContentView(
          state: state, search: search, selectedID: selected?.id, select: { selected = $0 })
      }
      .background(Surface.canvas)
    #endif
  }
  private func deviceButton(_ device: MeshDevice, own: Bool) -> some View {
    Button {
      selected = device
    } label: {
      DeviceRow(
        device: device, dnsName: state.dnsName(device),
        canonicalName: state.canonicalDnsName(device),
        isSelf: own,
        contact: own
          ? HeartbeatCaption(
            result: state.heartbeatResult, time: state.heartbeatTime,
            succeeded: state.heartbeatSucceeded) : nil)
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
  @State private var showNetwork = false
  public init(model: AppModel) { self.model = model }
  public var body: some View {
    VStack(spacing: 0) {
      MeshwayPanelHeader(
        title: DeviceScreenState(model: model).networkTitle
          == DeviceScreenState(model: model).serverHost
          ? "Meshway" : DeviceScreenState(model: model).networkTitle,
        subtitle: DeviceScreenState(model: model).serverHost,
        refreshing: model.enrollment.refreshing,
        refresh: model.enrollment.snapshot?.phase == .enrolled
          ? { Task { await model.enrollment.refresh() } } : nil
      ) {
        if let server = model.enrollment.snapshot?.server {
          Button("Network details") { showNetwork = true }
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
          state: DeviceScreenState(model: model), compact: true, selectedID: selected?.id,
          select: { selected = $0 }
        )
        .frame(height: LayoutMetrics.panelContentHeight)
      } else {
        VStack(spacing: 12) {
          MeshwayGlyph(size: 44)
          Text(model.status.title).font(.headline)
          if let message = model.enrollment.message {
            Text(message).font(.caption).foregroundStyle(.secondary)
          }
          Text(
            model.enrollment.snapshot?.phase == .revoked
              ? "Open Meshway to reset this device locally."
              : "Open Meshway to sign in with your browser."
          )
          .font(.callout).foregroundStyle(.secondary)
        }.multilineTextAlignment(.center).frame(maxWidth: .infinity)
          .padding(LayoutMetrics.inset)
      }
      Divider()
      HStack {
        Button("Open Meshway") { openWindow(id: "main") }
        Spacer()
        #if os(macOS)
          Button("Quit") { NSApplication.shared.terminate(nil) }
        #endif
      }.buttonStyle(.borderless).padding(LayoutMetrics.panelInset)
    }
    .frame(width: 360).background(Surface.canvas).tint(.indigo)
    .sheet(item: $selected) { device in
      DeviceDetailView(
        device: device, dnsName: DeviceScreenState(model: model).dnsName(device),
        canonicalName: DeviceScreenState(model: model).canonicalDnsName(device),
        isSelf: device.id == model.enrollment.snapshot?.node?.id)
    }
    .sheet(isPresented: $showNetwork) { NetworkDetailView(model: model) }
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
