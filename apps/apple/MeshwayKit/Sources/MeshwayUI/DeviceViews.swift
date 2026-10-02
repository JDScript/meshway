import Foundation
import MeshwayCore
import MeshwayModel
import SwiftUI

// Shared edges and an eight-point spacing rhythm for the window and panel.
enum LayoutMetrics {
  static let inset: CGFloat = 24
  static let panelInset: CGFloat = 16
  static let sectionGap: CGFloat = 16
  static let groupGap: CGFloat = 8
  static let rowInset: CGFloat = 16
  static let rowVerticalInset: CGFloat = 12
  static let rowHeight: CGFloat = 56
  static let iconSize: CGFloat = 28
  static let iconGap: CGFloat = 12
  static let countryWidth: CGFloat = 24
  static let textInset = rowInset + iconSize + iconGap
  static let cornerRadius: CGFloat = 12
  static let sheetWidth: CGFloat = 480
  static let sheetHeight: CGFloat = 560
  static let panelContentHeight: CGFloat = 336
  static let compactOnboardingHeight: CGFloat = 520
}

enum Surface {
  static var canvas: Color {
    #if os(macOS)
      Color(nsColor: .windowBackgroundColor)
    #else
      Color(uiColor: .systemGroupedBackground)
    #endif
  }
  static var card: Color {
    #if os(macOS)
      Color(nsColor: .controlBackgroundColor)
    #else
      Color(uiColor: .secondarySystemGroupedBackground)
    #endif
  }
}

public struct DeviceScreenState: Sendable {
  public var networkTitle: String
  public var serverHost: String?
  public var ownDevice: MeshDevice?
  public var peers: [MeshDevice]
  public var names: [MeshDnsName]
  public var loaded: Bool
  public var loading: Bool
  public var error: String?
  public var dnsError: String?
  public var heartbeatResult: String?
  public var heartbeatSucceeded: Bool?
  public var heartbeatTime: Date?
  public var refreshedAt: Date?
  public init(
    networkTitle: String, ownDevice: MeshDevice?, peers: [MeshDevice], names: [MeshDnsName] = [],
    loaded: Bool = true, loading: Bool = false, error: String? = nil, dnsError: String? = nil,
    heartbeatResult: String? = nil, heartbeatTime: Date? = nil, refreshedAt: Date? = nil,
    serverHost: String? = nil, heartbeatSucceeded: Bool? = nil
  ) {
    self.networkTitle = networkTitle
    self.serverHost = serverHost
    self.ownDevice = ownDevice
    self.peers = peers
    self.names = names
    self.loaded = loaded
    self.loading = loading
    self.error = error
    self.dnsError = dnsError
    self.heartbeatResult = heartbeatResult
    self.heartbeatSucceeded = heartbeatSucceeded
    self.heartbeatTime = heartbeatTime
    self.refreshedAt = refreshedAt
  }
  @MainActor init(model: AppModel) {
    let coordinator = model.enrollment
    let server = coordinator.snapshot?.server ?? model.controller?.url.absoluteString
    serverHost = server.flatMap { URL(string: $0)?.host }
    networkTitle =
      coordinator.dnsInventory?.zone ?? server.flatMap { URL(string: $0)?.host } ?? "Meshway"
    if let own = coordinator.inventory?.ownDevice {
      ownDevice = own
    } else if let node = coordinator.snapshot?.node {
      ownDevice = MeshDevice(
        id: node.id, name: node.name, country: nil, meshIPv4: node.meshIPv4, meshIPv6: node.meshIPv6
      )
    } else {
      ownDevice = nil
    }
    peers =
      coordinator.inventory?.peers.sorted {
        $0.name.localizedStandardCompare($1.name) == .orderedAscending
      } ?? []
    names = coordinator.dnsInventory?.names ?? []
    loaded = coordinator.inventory != nil
    loading = coordinator.refreshing
    error = coordinator.inventoryMessage
    dnsError = coordinator.dnsMessage
    heartbeatResult = coordinator.heartbeatResult
    heartbeatSucceeded = coordinator.heartbeatSucceeded
    heartbeatTime = coordinator.heartbeatTime
    refreshedAt = coordinator.inventoryTime
  }
  func dnsName(_ device: MeshDevice) -> String? { names.first { $0.nodeID == device.id }?.name }
  func canonicalDnsName(_ device: MeshDevice) -> String? {
    names.first { $0.nodeID == device.id }?.canonicalName
  }
  func filtered(_ search: String) -> [MeshDevice] {
    guard !search.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return peers }
    return peers.filter { device in
      [device.name, device.meshIPv4, device.meshIPv6, dnsName(device) ?? "", device.country ?? ""]
        .contains { $0.localizedStandardContains(search) }
    }
  }
}

public struct ConnectSwitch: View {
  public init() {}
  public var body: some View {
    HStack(spacing: LayoutMetrics.groupGap) {
      Text("Not connected").font(.caption).foregroundStyle(.secondary)
      Toggle("Connect", isOn: .constant(false)).labelsHidden()
        .toggleStyle(.switch).controlSize(.mini).disabled(true)
    }
    .fixedSize().accessibilityElement(children: .combine)
    .accessibilityLabel("Not connected — tunnel not implemented")
    .help("The tunnel is not implemented in this version.")
  }
}

public struct MeshwayPanelHeader<Actions: View>: View {
  let title: String
  let subtitle: String?
  let compact: Bool
  let refreshing: Bool
  let refresh: (() -> Void)?
  @ViewBuilder let actions: () -> Actions
  public init(
    title: String, subtitle: String? = nil, compact: Bool = true,
    refreshing: Bool = false, refresh: (() -> Void)? = nil,
    @ViewBuilder actions: @escaping () -> Actions
  ) {
    self.title = title
    self.subtitle = subtitle
    self.compact = compact
    self.refreshing = refreshing
    self.refresh = refresh
    self.actions = actions
  }
  public var body: some View {
    HStack(spacing: LayoutMetrics.panelInset) {
      VStack(alignment: .leading, spacing: 4) {
        Text(title).font(compact ? .headline : .title3.weight(.semibold)).lineLimit(1)
          .truncationMode(.middle).help(title)
        if let subtitle {
          Text("Server · \(subtitle)").font(.caption).foregroundStyle(.secondary).lineLimit(1)
            .truncationMode(.middle).help(subtitle)
        }
      }.frame(maxWidth: .infinity, alignment: .leading)
      HStack(spacing: LayoutMetrics.groupGap) {
        if let refresh {
          Button(action: refresh) { Image(systemName: "arrow.clockwise") }
            .buttonStyle(.plain).foregroundStyle(.secondary).frame(width: 24, height: 24)
            .disabled(refreshing).accessibilityLabel("Refresh devices")
            .help("Refresh devices and contact the server")
        }
        Menu(content: actions) { Image(systemName: "ellipsis.circle").font(.body) }
          .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
          .tint(Color.secondary)
          .foregroundStyle(.secondary).accessibilityLabel("Network actions")
          .help("Network details, server URL and local reset")
        ConnectSwitch()
      }
    }.padding(.horizontal, compact ? LayoutMetrics.panelInset : LayoutMetrics.inset)
      .padding(.vertical, LayoutMetrics.panelInset)
  }
}

public struct NetworkLabel: View {
  let title: String
  public init(title: String) { self.title = title }
  public var body: some View {
    Text(title).font(.headline).lineLimit(1).help(title)
  }
}

struct TunnelNotice: View {
  var body: some View {
    Label("Tunnel not implemented in this version", systemImage: "info.circle")
      .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
  }
}

public struct SearchBar: View {
  @Binding var text: String
  public init(text: Binding<String>) { _text = text }
  public var body: some View {
    HStack(spacing: 8) {
      Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
      TextField("Name, IP or DNS name", text: $text).textFieldStyle(.plain).accessibilityLabel(
        "Search devices")
      if !text.isEmpty {
        Button {
          text = ""
        } label: {
          Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
        }
        .buttonStyle(.plain).accessibilityLabel("Clear search")
      }
    }.padding(.horizontal, LayoutMetrics.rowInset).padding(.vertical, LayoutMetrics.groupGap)
      .frame(minHeight: 32)
      .background(Surface.card, in: RoundedRectangle(cornerRadius: LayoutMetrics.groupGap))
      .overlay {
        RoundedRectangle(cornerRadius: LayoutMetrics.groupGap).strokeBorder(.primary.opacity(0.1))
      }
  }
}

public struct DeviceContentView: View {
  let state: DeviceScreenState
  let search: String
  let compact: Bool
  let selectedID: String?
  let select: ((MeshDevice) -> Void)?
  public init(
    state: DeviceScreenState, search: String = "", compact: Bool = false, selectedID: String? = nil,
    select: ((MeshDevice) -> Void)? = nil
  ) {
    self.state = state
    self.search = search
    self.compact = compact
    self.selectedID = selectedID
    self.select = select
  }
  public var body: some View {
    ScrollView {
      content
    }.background(Surface.canvas)
  }
  public var content: some View {
    VStack(
      alignment: .leading, spacing: compact ? LayoutMetrics.panelInset : LayoutMetrics.sectionGap
    ) {
      if let error = state.error, state.heartbeatSucceeded != false {
        RefreshError(message: error, hasInventory: state.loaded)
      }
      if let own = state.ownDevice {
        VStack(alignment: .leading, spacing: 8) {
          sectionHeading("This device")
          row(own, isSelf: true).deviceCard()
        }
      }
      VStack(alignment: .leading, spacing: 8) {
        HStack(spacing: LayoutMetrics.groupGap) {
          sectionHeading("Devices")
          Text("\(state.peers.count)").font(.caption.monospacedDigit()).foregroundStyle(.tertiary)
          Spacer()
          if state.loading && state.loaded {
            ActivityIndicator().accessibilityLabel("Refreshing devices")
          }
          InventoryUpdated(state: state)
        }
        VStack(spacing: 0) {
          let devices = Array(state.filtered(search).prefix(compact ? 3 : 1024))
          if devices.isEmpty {
            DeviceEmptyState(state: state, search: search).frame(maxWidth: .infinity).padding(24)
          }
          ForEach(devices) { device in
            row(device, isSelf: false)
            if device.id != devices.last?.id {
              Divider().padding(.leading, LayoutMetrics.textInset)
            }
          }
        }.deviceCard()
        if compact && state.peers.count > 3 {
          Text("\(state.peers.count - 3) more in Meshway").font(.caption).foregroundStyle(
            .secondary)
        }
      }
      if let error = state.dnsError {
        Text(error).font(.caption).foregroundStyle(.orange)
      }
    }.padding(.horizontal, compact ? LayoutMetrics.panelInset : LayoutMetrics.inset)
      .padding(.vertical, LayoutMetrics.panelInset)
  }
  private func sectionHeading(_ title: String) -> some View {
    Text(title).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
  }
  @ViewBuilder private func row(_ device: MeshDevice, isSelf: Bool) -> some View {
    let content = DeviceRow(
      device: device, dnsName: state.dnsName(device), canonicalName: state.canonicalDnsName(device),
      isSelf: isSelf,
      contact: isSelf
        ? HeartbeatCaption(
          result: state.heartbeatResult, time: state.heartbeatTime,
          succeeded: state.heartbeatSucceeded) : nil
    )
    .padding(.horizontal, LayoutMetrics.rowInset).padding(.vertical, LayoutMetrics.rowVerticalInset)
    .frame(minHeight: LayoutMetrics.rowHeight)
    .contentShape(Rectangle())
    if let select {
      DeviceRowButton(selected: selectedID == device.id, action: { select(device) }) { content }
        .contextMenu { CopyDeviceActions(device: device, dnsName: state.dnsName(device)) }
    } else {
      content.contextMenu { CopyDeviceActions(device: device, dnsName: state.dnsName(device)) }
    }
  }
}

extension View {
  fileprivate func deviceCard() -> some View {
    background(Surface.card)
      .clipShape(RoundedRectangle(cornerRadius: LayoutMetrics.cornerRadius))
      .overlay {
        RoundedRectangle(cornerRadius: LayoutMetrics.cornerRadius).strokeBorder(
          .primary.opacity(0.08))
      }
  }
}

private struct DeviceRowButton<Content: View>: View {
  let selected: Bool
  let action: () -> Void
  @ViewBuilder let content: () -> Content
  @State private var hovering = false
  var body: some View {
    Button(action: action) {
      content().frame(maxWidth: .infinity, alignment: .leading)
        .background(
          selected ? Color.indigo.opacity(0.1) : Color.primary.opacity(hovering ? 0.04 : 0))
    }.buttonStyle(.plain)
      #if os(macOS)
        .onHover { hovering = $0 }
      #endif
  }
}

struct DeviceRow: View {
  let device: MeshDevice
  let dnsName: String?
  var canonicalName: String? = nil
  let isSelf: Bool
  var contact: HeartbeatCaption? = nil
  @Environment(\.dynamicTypeSize) private var dynamicType
  @ScaledMetric(relativeTo: .caption2) private var countryWidth = LayoutMetrics.countryWidth
  var body: some View {
    HStack(alignment: .center, spacing: LayoutMetrics.iconGap) {
      DeviceAvatar(device: device, isSelf: isSelf)
      VStack(alignment: .leading, spacing: 4) {
        HStack(alignment: .firstTextBaseline, spacing: LayoutMetrics.groupGap) {
          if dynamicType.isAccessibilitySize {
            VStack(alignment: .leading, spacing: 4) {
              deviceName.lineLimit(3)
              address
            }.frame(maxWidth: .infinity, alignment: .leading)
          } else {
            deviceName.lineLimit(1).truncationMode(.tail)
              .frame(maxWidth: .infinity, alignment: .leading)
            address.fixedSize()
          }
          countryLabel
        }
        if let contact {
          contact
        } else if let friendlyName {
          Text(friendlyName).font(.caption).foregroundStyle(.secondary)
            .lineLimit(1).truncationMode(.middle).help(friendlyName)
        }
      }
    }.accessibilityElement(children: .combine)
      .accessibilityHint("Device details and copy actions")
  }
  private var deviceName: some View {
    Text(device.name).font(.body.weight(.medium)).foregroundStyle(.primary).help(device.name)
  }
  private var friendlyName: String? {
    guard let dnsName, dnsName != canonicalName else { return nil }
    // Older test/preview callers do not carry a separate canonical name.
    let labels = dnsName.split(separator: ".")
    if labels.count > 2, labels[0] == "id",
      labels[1].allSatisfy({ $0.isHexDigit || $0 == "-" })
    {
      return nil
    }
    return dnsName
  }
  private var address: some View {
    Text(device.meshIPv4).font(.callout.monospaced()).foregroundStyle(.secondary)
  }
  private var countryLabel: some View {
    Text(device.country ?? "").font(.caption2.weight(.medium)).foregroundStyle(.secondary)
      .frame(width: countryWidth, alignment: .trailing)
      .help(device.country.map { "Administrator-assigned country: \($0)" } ?? "")
      .accessibilityLabel(device.country.map { "Administrator-assigned country \($0)" } ?? "")
      .accessibilityHidden(device.country == nil)
  }
}

struct HeartbeatCaption: View {
  let result: String?
  let time: Date?
  var succeeded: Bool? = nil
  var body: some View {
    Group {
      if succeeded == false {
        Text("Contact failed: \(result ?? "The server could not be reached.")")
          .foregroundStyle(.orange)
      } else if succeeded == true, let time {
        HStack(spacing: 4) {
          Text("Last contact with server")
          Text(time, format: .dateTime.hour().minute())
        }.foregroundStyle(.secondary)
      } else {
        Text("Not contacted yet").foregroundStyle(.secondary)
      }
    }.font(.caption).fixedSize(horizontal: false, vertical: true)
  }
}

struct RefreshError: View {
  let message: String
  let hasInventory: Bool
  var body: some View {
    HStack(alignment: .top, spacing: 10) {
      Image(systemName: "exclamationmark.circle").foregroundStyle(.orange)
      VStack(alignment: .leading, spacing: 4) {
        Text("Unable to refresh devices").font(.callout.weight(.semibold))
        Text(message).font(.caption).foregroundStyle(.secondary)
        if hasInventory {
          Text("Showing the last fetched inventory.").font(.caption).foregroundStyle(.secondary)
        }
      }
    }.padding(14).frame(maxWidth: .infinity, alignment: .leading)
      .background(Color.orange.opacity(0.07), in: RoundedRectangle(cornerRadius: 12))
  }
}
struct DeviceEmptyState: View {
  let state: DeviceScreenState
  let search: String
  var body: some View {
    VStack(spacing: 8) {
      if state.loading && !state.loaded {
        ActivityIndicator()
      } else {
        Image(systemName: search.isEmpty ? "square.stack.3d.up" : "magnifyingglass").font(.title2)
          .foregroundStyle(.tertiary)
      }
      Text(
        !search.isEmpty
          ? "No matching devices"
          : state.loaded
            ? "No other devices yet" : state.loading ? "Loading devices…" : "Devices not loaded"
      )
      .font(.callout.weight(.medium))
      Text(
        !search.isEmpty
          ? "Try another name or address."
          : state.loaded
            ? "Other nodes in this network will appear here."
            : state.loading
              ? "Reading this network’s inventory from the server."
              : "Refresh to fetch this network’s inventory."
      )
      .font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.center)
    }
  }
}
struct InventoryUpdated: View {
  let state: DeviceScreenState
  var body: some View {
    Group {
      if let time = state.refreshedAt {
        HStack(spacing: 4) {
          Text(state.error == nil ? "Updated" : "Previous fetch")
          Text(time, format: .dateTime.hour().minute())
        }
      }
    }.font(.caption2).foregroundStyle(state.error == nil ? Color.secondary : Color.orange)
      .help(
        state.error == nil ? "Last device inventory fetch" : "Showing the last fetched inventory")
  }
}

struct ActivityIndicator: View {
  @State private var rotating = false
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  var body: some View {
    Circle().trim(from: 0.08, to: 0.8).stroke(
      .secondary, style: StrokeStyle(lineWidth: 1.8, lineCap: .round)
    )
    .frame(width: 16, height: 16).rotationEffect(.degrees(rotating ? 360 : 0))
    .onAppear {
      guard !reduceMotion else { return }
      withAnimation(.linear(duration: 1).repeatForever(autoreverses: false)) { rotating = true }
    }
    .onDisappear { rotating = false }
    .accessibilityLabel("Loading")
  }
}

enum Clipboard {
  static func copy(_ value: String) {
    #if os(macOS)
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(value, forType: .string)
    #else
      UIPasteboard.general.string = value
    #endif
  }
}
struct CopyDeviceActions: View {
  let device: MeshDevice
  let dnsName: String?
  var body: some View {
    Button("Copy IPv4", systemImage: "doc.on.doc") { Clipboard.copy(device.meshIPv4) }
    Button("Copy IPv6") { Clipboard.copy(device.meshIPv6) }
    if let dnsName { Button("Copy DNS name") { Clipboard.copy(dnsName) } }
    Button("Copy node ID") { Clipboard.copy(device.id) }
  }
}

struct DeviceDetailView: View {
  let device: MeshDevice
  let dnsName: String?
  var canonicalName: String? = nil
  let isSelf: Bool
  @Environment(\.dismiss) private var dismiss
  var body: some View {
    NavigationStack {
      Form {
        Section(isSelf ? "This device" : "Device") {
          Text(device.name).font(.title3.weight(.semibold))
          copyRow("Mesh IPv4", device.meshIPv4)
          copyRow("Mesh IPv6", device.meshIPv6)
          if let canonicalName { copyRow("Canonical DNS name", canonicalName) }
          if let dnsName, dnsName != canonicalName { copyRow("DNS alias", dnsName) }
          if let country = device.country { LabeledContent("Admin country", value: country) }
          copyRow("Node ID", device.id)
        }
        Section {
          Text("These are assigned addresses, not a connectivity test.").foregroundStyle(.secondary)
        }
      }.formStyle(.grouped).navigationTitle("Device details")
        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
    }
    #if os(macOS)
      .frame(width: LayoutMetrics.sheetWidth, height: LayoutMetrics.sheetHeight)
    #endif
    #if DEBUG && os(macOS)
      .modifier(SnapshotSheetDismissal(dismiss: dismiss))
    #endif
  }
  private func copyRow(_ title: String, _ value: String) -> some View {
    VStack(alignment: .leading, spacing: 4) {
      Text(title).font(.caption).foregroundStyle(.secondary)
      HStack(alignment: .top) {
        Text(value).font(.callout.monospaced()).textSelection(.enabled)
        Spacer()
        Button {
          Clipboard.copy(value)
        } label: {
          Image(systemName: "doc.on.doc")
        }
        .buttonStyle(.borderless).accessibilityLabel("Copy \(title)")
      }
    }.padding(.vertical, 4)
  }
}

struct NetworkDetailView: View {
  let model: AppModel
  @Environment(\.dismiss) private var dismiss
  var body: some View {
    NavigationStack {
      Form {
        Section("Current network") {
          LabeledContent(
            "Server", value: model.enrollment.snapshot?.server ?? model.controllerDraft)
          if let zone = model.enrollment.dnsInventory?.zone {
            LabeledContent("DNS zone", value: zone)
          }
          if let inventory = model.enrollment.inventory {
            LabeledContent("IPv4 prefix", value: inventory.ipv4Prefix)
            LabeledContent("IPv6 prefix", value: inventory.ipv6Prefix)
            LabeledContent("Revision", value: String(inventory.revision))
          }
          Text(
            model.enrollment.snapshot == nil
              ? "No saved network is selected."
              : "Only the selected enrollment is active. Saved networks are available in the ⋯ menu."
          )
          .font(.callout).foregroundStyle(.secondary)
        }
        Section("About") {
          LabeledContent("Rust core", value: model.coreInfo.version)
          Text(model.preferencesDiagnostic).font(.caption).foregroundStyle(.secondary)
          Button("Inspect system VPN preferences") { Task { await model.inspectPreferences() } }
        }
      }.formStyle(.grouped).navigationTitle("Network details")
        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
    }
    #if os(macOS)
      .frame(width: LayoutMetrics.sheetWidth, height: LayoutMetrics.sheetHeight)
    #endif
    #if DEBUG && os(macOS)
      .modifier(SnapshotSheetDismissal(dismiss: dismiss))
    #endif
  }
}
