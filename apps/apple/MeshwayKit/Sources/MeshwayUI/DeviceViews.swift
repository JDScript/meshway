import Foundation
import MeshwayCore
import MeshwayModel
import SwiftUI

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
  public var ownDevice: MeshDevice?
  public var peers: [MeshDevice]
  public var names: [MeshDnsName]
  public var loaded: Bool
  public var loading: Bool
  public var error: String?
  public var dnsError: String?
  public var heartbeatResult: String?
  public var heartbeatTime: Date?
  public var refreshedAt: Date?
  public init(
    networkTitle: String, ownDevice: MeshDevice?, peers: [MeshDevice], names: [MeshDnsName] = [],
    loaded: Bool = true, loading: Bool = false, error: String? = nil, dnsError: String? = nil,
    heartbeatResult: String? = nil, heartbeatTime: Date? = nil, refreshedAt: Date? = nil
  ) {
    self.networkTitle = networkTitle
    self.ownDevice = ownDevice
    self.peers = peers
    self.names = names
    self.loaded = loaded
    self.loading = loading
    self.error = error
    self.dnsError = dnsError
    self.heartbeatResult = heartbeatResult
    self.heartbeatTime = heartbeatTime
    self.refreshedAt = refreshedAt
  }
  @MainActor init(model: AppModel) {
    let coordinator = model.enrollment
    let server = coordinator.snapshot?.server ?? model.controller?.url.absoluteString
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
    heartbeatTime = coordinator.heartbeatTime
    refreshedAt = coordinator.inventoryTime
  }
  func dnsName(_ device: MeshDevice) -> String? { names.first { $0.nodeID == device.id }?.name }
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
    Toggle("Connect", isOn: .constant(false)).toggleStyle(.switch).disabled(true)
      .fixedSize().accessibilityLabel("Connect — tunnel not implemented")
      .help("The tunnel is not implemented in this version.")
  }
}

public struct MeshwayPanelHeader<Actions: View>: View {
  let title: String
  @ViewBuilder let actions: () -> Actions
  public init(title: String, @ViewBuilder actions: @escaping () -> Actions) {
    self.title = title
    self.actions = actions
  }
  public var body: some View {
    HStack(spacing: 12) {
      Menu(content: actions) {
        NetworkLabel(title: title)
      }.menuStyle(.borderlessButton).accessibilityLabel("Select network")
      Spacer(minLength: 8)
      ConnectSwitch()
    }.padding(18)
  }
}

public struct NetworkLabel: View {
  let title: String
  public init(title: String) { self.title = title }
  public var body: some View {
    HStack(spacing: 6) {
      Image(systemName: "network").foregroundStyle(.indigo)
      Text(title).font(.headline).lineLimit(1)
      Image(systemName: "chevron.down").font(.caption2).foregroundStyle(.secondary)
    }
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
    }.padding(10).background(Surface.card, in: RoundedRectangle(cornerRadius: 10))
      .overlay { RoundedRectangle(cornerRadius: 10).strokeBorder(.primary.opacity(0.07)) }
  }
}

public struct DeviceContentView: View {
  let state: DeviceScreenState
  let search: String
  let compact: Bool
  let select: ((MeshDevice) -> Void)?
  public init(
    state: DeviceScreenState, search: String = "", compact: Bool = false,
    select: ((MeshDevice) -> Void)? = nil
  ) {
    self.state = state
    self.search = search
    self.compact = compact
    self.select = select
  }
  public var body: some View {
    ScrollView {
      content
    }.background(Surface.canvas)
  }
  public var content: some View {
    VStack(alignment: .leading, spacing: compact ? 16 : 22) {
      TunnelNotice()
      if let error = state.error { RefreshError(message: error, hasInventory: state.loaded) }
      if let own = state.ownDevice {
        VStack(alignment: .leading, spacing: 8) {
          sectionHeading("This device")
          VStack(alignment: .leading, spacing: 0) {
            row(own, isSelf: true)
            Divider().padding(.leading, 58)
            HeartbeatCaption(result: state.heartbeatResult, time: state.heartbeatTime)
              .padding(.horizontal, 16).padding(.vertical, 12)
          }.background(Surface.card, in: RoundedRectangle(cornerRadius: 14))
            .overlay { RoundedRectangle(cornerRadius: 14).strokeBorder(.primary.opacity(0.07)) }
        }
      }
      VStack(alignment: .leading, spacing: 8) {
        HStack {
          sectionHeading("Devices")
          Spacer()
          if state.loading && state.loaded {
            ActivityIndicator().accessibilityLabel("Refreshing devices")
          }
          Text("\(state.peers.count)").font(.caption.monospacedDigit()).foregroundStyle(
            .secondary)
        }
        VStack(spacing: 0) {
          let devices = Array(state.filtered(search).prefix(compact ? 3 : 1024))
          if devices.isEmpty {
            DeviceEmptyState(state: state, search: search).frame(maxWidth: .infinity).padding(24)
          }
          ForEach(devices) { device in
            row(device, isSelf: false)
            if device.id != devices.last?.id { Divider().padding(.leading, 58) }
          }
        }.background(Surface.card, in: RoundedRectangle(cornerRadius: 14))
          .overlay { RoundedRectangle(cornerRadius: 14).strokeBorder(.primary.opacity(0.07)) }
        if compact && state.peers.count > 3 {
          Text("\(state.peers.count - 3) more in Meshway").font(.caption).foregroundStyle(
            .secondary)
        }
      }
      if !compact { InventoryFooter(state: state) }
    }.padding(compact ? 18 : 24).padding(.top, compact ? 0 : -6)
  }
  private func sectionHeading(_ title: String) -> some View {
    Text(title).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary).padding(
      .leading, 2)
  }
  @ViewBuilder private func row(_ device: MeshDevice, isSelf: Bool) -> some View {
    let content = DeviceRow(
      device: device, dnsName: state.dnsName(device), isSelf: isSelf, compact: compact
    )
    .padding(.horizontal, 14).padding(.vertical, compact ? 12 : 14).contentShape(Rectangle())
    if let select {
      Button {
        select(device)
      } label: {
        content
      }.buttonStyle(.plain)
        .contextMenu { CopyDeviceActions(device: device, dnsName: state.dnsName(device)) }
    } else {
      content.contextMenu { CopyDeviceActions(device: device, dnsName: state.dnsName(device)) }
    }
  }
}

struct DeviceRow: View {
  let device: MeshDevice
  let dnsName: String?
  let isSelf: Bool
  var compact = false
  var body: some View {
    HStack(alignment: .top, spacing: 12) {
      Image(systemName: "circle.hexagongrid")
        .font(.system(size: compact ? 17 : 20, weight: .medium))
        .foregroundStyle(isSelf ? Color.indigo : Color.secondary)
        .frame(width: 34, height: 36)
        .background(
          isSelf ? Color.indigo.opacity(0.09) : Color.secondary.opacity(0.07),
          in: RoundedRectangle(cornerRadius: 9)
        )
        .accessibilityHidden(true)
      VStack(alignment: .leading, spacing: 4) {
        HStack(alignment: .firstTextBaseline) {
          Text(device.name).font(.body.weight(.medium)).foregroundStyle(.primary).lineLimit(2)
          Spacer(minLength: 4)
          if let country = device.country {
            Text(country).font(.caption2.weight(.medium)).foregroundStyle(.secondary)
              .help("Administrator-assigned country: \(country)")
              .accessibilityLabel("Administrator-assigned country \(country)")
          }
        }
        Text(device.meshIPv4).font(.callout.monospaced()).foregroundStyle(.secondary)
        if !compact {
          if let dnsName { Text(dnsName).font(.caption).foregroundStyle(.secondary).lineLimit(2) }
          Text(device.meshIPv6).font(.caption.monospaced()).foregroundStyle(.secondary).opacity(0.8)
            .lineLimit(2)
        }
      }
    }.accessibilityElement(children: .combine)
      .accessibilityHint("Device details and copy actions")
  }
}

struct HeartbeatCaption: View {
  let result: String?
  let time: Date?
  var body: some View {
    VStack(alignment: .leading, spacing: 5) {
      if let result {
        Text(result).font(.caption).foregroundStyle(.secondary)
      } else {
        Text("Controller has not been contacted yet.").font(.caption).foregroundStyle(.secondary)
      }
      if let time {
        HStack(spacing: 4) {
          Text("Last heartbeat")
          Text(time, format: .dateTime.hour().minute().second())
        }.font(.caption2).foregroundStyle(.secondary)
      }
    }.fixedSize(horizontal: false, vertical: true)
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
struct InventoryFooter: View {
  let state: DeviceScreenState
  var body: some View {
    VStack(alignment: .leading, spacing: 5) {
      if let error = state.dnsError { Text(error).foregroundStyle(.secondary) }
      if let time = state.refreshedAt {
        HStack(spacing: 4) {
          Text("Inventory fetched")
          Text(time, format: .dateTime.hour().minute())
        }
      }
      Text("Device inventory does not indicate connectivity.")
    }.font(.caption2).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
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
  let isSelf: Bool
  @Environment(\.dismiss) private var dismiss
  var body: some View {
    NavigationStack {
      Form {
        Section(isSelf ? "This device" : "Device") {
          Text(device.name).font(.title3.weight(.semibold))
          copyRow("Mesh IPv4", device.meshIPv4)
          copyRow("Mesh IPv6", device.meshIPv6)
          if let dnsName { copyRow("DNS name", dnsName) }
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
      .frame(width: 480, height: 430)
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
          Text("This device belongs to one network. Additional networks are not supported yet.")
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
      .frame(width: 480, height: 450)
    #endif
  }
}
