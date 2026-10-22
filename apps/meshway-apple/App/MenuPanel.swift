import SwiftUI

struct MenuPanel: View {
    let model: AppModel
    @Environment(\.openWindow) private var openWindow
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        GlassEffectContainer(spacing: 12) {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(URL(string: model.status?.server ?? model.server)?.host ?? "Meshway").font(.caption.weight(.medium)).foregroundStyle(.secondary).lineLimit(1)
                Spacer()
                if model.sampleMode { SampleBanner() }
            }.frame(height: 24)
            ConnectionHeader(model: model, compact: true).padding(.vertical, 8)
            HStack {
                Text(model.status?.node?.ipv4 ?? model.status?.node?.ipv6 ?? "No installed address")
                    .font(model.status?.node == nil ? .caption : .caption.monospaced()).foregroundStyle(.secondary)
                Spacer()
                if let address = model.status?.node?.ipv4 ?? model.status?.node?.ipv6 { CopyButton(value: address) }
            }.frame(height: 24)
            Divider().padding(.vertical, 10)
            HStack {
                Label("Exit profile", systemImage: "arrow.up.forward.circle").foregroundStyle(.secondary)
                Spacer()
                if let selection = model.selection { ProfileMenu(model: model, selection: selection) }
                else { Text("Unavailable").foregroundStyle(.tertiary) }
            }.font(.callout).frame(height: 28)
            HStack {
                Text("PEERS").font(.system(size: 10, weight: .semibold)).tracking(0.8)
                Spacer()
                Text(model.status.map { String($0.peers.count) } ?? "—").font(.caption).contentTransition(.numericText())
            }.foregroundStyle(.secondary).padding(.top, 18).padding(.bottom, 6)
            ZStack {
                ScrollView {
                    LazyVStack(spacing: 2) {
                        ForEach(model.status?.peers ?? [], id: \.nodeId) { peer in
                            CompactPeerRow(model: model, peer: peer).contextMenu { PeerCopyActions(peer: peer) }
                        }
                    }.padding(.horizontal, 1)
                }.scrollIndicators(.hidden)
                    .animation(reduceMotion ? nil : .smooth(duration: 0.22), value: model.status?.peers.map(\.nodeId))
                if model.status?.peers.isEmpty != false {
                    VStack(spacing: 8) {
                        Image(systemName: model.status?.phase == .awaitingApproval ? "person.badge.key" : "desktopcomputer").font(.title2).foregroundStyle(.tertiary)
                            .symbolEffect(.pulse, isActive: model.status?.phase == .awaitingApproval && !reduceMotion)
                        Text(model.status?.phase == .awaitingApproval ? "Waiting for browser approval" : "Your peers will appear here").font(.callout).foregroundStyle(.secondary)
                        Text("Open Meshway for connection details.").font(.caption).foregroundStyle(.tertiary)
                    }
                }
            }.frame(height: 172)
            FeedbackShelf(model: model, compact: true)
            Divider().padding(.bottom, 10)
            HStack {
                Button("Open Meshway", systemImage: "arrow.up.forward.app") { openWindow(id: "main"); NSApp.activate() }.buttonStyle(.glass)
                Spacer()
                Menu {
                    Button("Settings…") { AppNavigation.shared.page = .device; openWindow(id: "main"); NSApp.activate() }
                    Divider()
                    Button("Quit Meshway") { NSApp.terminate(nil) }
                } label: { Image(systemName: "gearshape") }.menuStyle(.borderlessButton).fixedSize().help("Meshway settings")
            }.frame(height: 30)
        }.padding(16).frame(width: 344).background(.regularMaterial)
        }
    }
}

struct ProfileMenu: View {
    let model: AppModel
    let selection: ClientSelection
    var body: some View {
        Menu {
            ForEach(selection.profiles, id: \.name) { profile in
                Button { Task { await model.perform(.selectProfile(profile: profile.name)) } } label: {
                    if profile.name == selection.appliedProfile { Label(profile.name, systemImage: "checkmark") }
                    else { Text(profile.name) }
                }.disabled(!model.canRequestSelection || selection.locked || model.busy || model.sampleMode)
            }
            Divider()
            Button("Use default · \(selection.defaultProfile)") { Task { await model.perform(.clearProfile) } }
                .disabled(!model.canRequestSelection || model.busy || model.sampleMode || selection.requestedProfile == nil)
        } label: {
            HStack(spacing: 4) {
                Text(selection.appliedProfile).lineLimit(1)
                if !model.netmapApplied { Image(systemName: "clock").help("Netmap selection is not yet applied") }
            }
        }.menuStyle(.borderlessButton).fixedSize()
            .help(unavailableHelp(model, fallback: selection.locked ? "Profile selection is locked by your administrator" : "Select an exit profile"))
    }
}
