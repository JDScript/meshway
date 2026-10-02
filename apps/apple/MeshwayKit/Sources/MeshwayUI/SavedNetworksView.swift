import MeshwayModel
import SwiftUI

private func savedNetworkSubtitle(_ profile: SavedEnrollment, among profiles: [SavedEnrollment])
  -> String
{
  var subtitle = profile.serverHost + (profile.ipv4Prefix.map { " · " + $0 } ?? "")
  if profiles.contains(where: {
    $0.id != profile.id && $0.serverHost == profile.serverHost && $0.label == profile.label
      && $0.ipv4Prefix == profile.ipv4Prefix
  }), let nodeID = profile.snapshot?.node?.id {
    subtitle += " · Node \(nodeID.prefix(8))"
  }
  return subtitle
}

struct SavedNetworkLabel: View {
  let profile: SavedEnrollment
  let subtitle: String
  let active: Bool
  var body: some View {
    Label {
      Text(profile.label + (profile.badge.map { " · " + $0 } ?? ""))
      Text(subtitle)
    } icon: {
      if active { Image(systemName: "checkmark") }
    }
  }
}

struct NetworkActions: View {
  let model: AppModel
  let details: () -> Void
  let remove: () -> Void
  var showSignIn: () -> Void = {}
  var body: some View {
    if !model.enrollment.profiles.isEmpty {
      Section("Saved networks") {
        ForEach(model.enrollment.profiles) { profile in
          Button {
            Task { await model.switchNetwork(profile.id) }
          } label: {
            SavedNetworkLabel(
              profile: profile,
              subtitle: savedNetworkSubtitle(profile, among: model.enrollment.profiles),
              active: profile.id == model.enrollment.activeProfileID)
          }
        }
      }
      Divider()
    }
    Button("Add network…", systemImage: "plus") {
      showSignIn()
      Task {
        await model.addNetwork()
        await model.signIn()
      }
    }
    if let server = model.enrollment.snapshot?.server ?? model.controller?.url.absoluteString {
      Button("Network details…", systemImage: "info.circle", action: details)
      Button("Copy server URL", systemImage: "doc.on.doc") { Clipboard.copy(server) }
    }
    if model.enrollment.activeProfileID != nil || model.enrollment.hasIdentity {
      Divider()
      Button(
        "Remove this network from device…", systemImage: "minus.circle", role: .destructive,
        action: remove)
    }
  }
}

/// Also used by the debug window harness when the OS-owned popup is not capturable.
/// Entries and actions are the same live saved enrollments as the ⋯ menu.
public struct SavedNetworksView: View {
  private let model: AppModel
  @State private var showDetails = false
  @State private var confirmRemove = false
  public init(model: AppModel) { self.model = model }
  public var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      Text("Saved networks").font(.headline)
      ForEach(model.enrollment.profiles) { profile in
        Button {
          Task { await model.switchNetwork(profile.id) }
        } label: {
          HStack(spacing: 12) {
            Image(
              systemName: profile.id == model.enrollment.activeProfileID ? "checkmark" : "circle"
            )
            .frame(width: 18).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 4) {
              Text(profile.label).fontWeight(.medium)
              Text(savedNetworkSubtitle(profile, among: model.enrollment.profiles))
                .font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            if let badge = profile.badge { Text(badge).font(.caption).foregroundStyle(.secondary) }
          }.frame(maxWidth: .infinity, alignment: .leading).padding(12)
            .background(Surface.card, in: RoundedRectangle(cornerRadius: 10))
        }.buttonStyle(.plain)
      }
      Button("Add network…", systemImage: "plus") {
        Task {
          await model.addNetwork()
          await model.signIn()
        }
      }
      .buttonStyle(.borderless)
      if model.enrollment.activeProfileID != nil {
        Divider()
        Button("Network details…", systemImage: "info.circle") { showDetails = true }
          .buttonStyle(.borderless)
        if let server = model.enrollment.snapshot?.server {
          Button("Copy server URL", systemImage: "doc.on.doc") { Clipboard.copy(server) }
            .buttonStyle(.borderless)
        }
        Button("Remove this network from device…", systemImage: "minus.circle", role: .destructive)
        {
          confirmRemove = true
        }.buttonStyle(.borderless)
      }
    }.disabled(model.enrollment.switching)
      .padding(LayoutMetrics.inset).background(Surface.canvas)
      .sheet(isPresented: $showDetails) { NetworkDetailView(model: model) }
      .onChange(of: model.enrollment.activeProfileID) {
        showDetails = false
        confirmRemove = false
      }
      .confirmationDialog("Remove this network from device?", isPresented: $confirmRemove) {
        Button("Remove local enrollment", role: .destructive) {
          Task { await model.removeNetwork() }
        }
      } message: {
        Text("This removes only the local enrollment. It does not revoke the server-side node.")
      }
  }
}
