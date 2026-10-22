import SwiftUI

enum AppPage: String, CaseIterable, Identifiable {
    case overview, peers, exit, diagnostics, device
    var id: String { rawValue }
    var title: String {
        switch self {
        case .overview: "Overview"
        case .peers: "Peers"
        case .exit: "Exit profiles"
        case .diagnostics: "Diagnostics"
        case .device: "This Mac"
        }
    }
    var symbol: String {
        switch self {
        case .overview: "network"
        case .peers: "desktopcomputer"
        case .exit: "arrow.up.forward.circle"
        case .diagnostics: "waveform.path.ecg"
        case .device: "laptopcomputer"
        }
    }
}

struct MainView: View {
    @Bindable var model: AppModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Bindable private var navigation = AppNavigation.shared
    private var page: AppPage? {
        get { navigation.page }
        nonmutating set { navigation.page = newValue }
    }
    @State private var selectedPeer: String?
    @State private var showLogout = false

    private var needsEnrollment: Bool {
        model.status?.phase == .awaitingApproval ||
        (model.status?.identity == nil && (model.status == nil || model.status?.phase == .down))
    }

    var body: some View {
        NavigationSplitView {
            VStack(spacing: 0) {
                HStack(spacing: 10) {
                    Image(systemName: "network").font(.title2).foregroundStyle(.tint)
                    Text("Meshway").font(.title3.weight(.semibold))
                    Spacer()
                }.padding(20)
                List(AppPage.allCases, selection: $navigation.page) { item in
                    Label(item.title, systemImage: item.symbol).tag(item)
                }.listStyle(.sidebar)
                VStack(alignment: .leading, spacing: 8) {
                    PhaseLabel(model: model)
                    Text(model.status?.node?.hostname ?? "This Mac").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    if model.sampleMode { SampleBanner() }
                }.frame(maxWidth: .infinity, alignment: .leading).padding(20)
            }.navigationSplitViewColumnWidth(min: 190, ideal: 210, max: 240)
        } detail: {
            ZStack {
                Color(nsColor: .windowBackgroundColor)
                Group {
                    if needsEnrollment && page != .device { EnrollmentView(model: model) }
                    else { detail.id(page).transition(.opacity) }
                }.animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: page)
                    .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: needsEnrollment)
            }
            .safeAreaInset(edge: .bottom, spacing: 0) { FeedbackShelf(model: model).background(.bar) }
            .navigationTitle(needsEnrollment && page != .device ? "Meshway" : (page ?? .overview).title)
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button("Refresh", systemImage: "arrow.clockwise") { Task { await model.refresh() } }
                        .disabled(model.busy || model.sampleMode)
                        .help(unavailableHelp(model, fallback: "Read the latest Node status"))
                }
            }
        }
        .task {
            #if DEBUG
            if model.sampleMode, let section = model.previewSection { page = AppPage(rawValue: section) ?? .overview }
            #endif
        }
        .confirmationDialog("Permanently remove this node from its network?", isPresented: $showLogout, titleVisibility: .visible) {
            Button("Log out", role: .destructive) { Task { await model.identityOperation(logout: true) } }
            Button("Cancel", role: .cancel) {}
        } message: { Text("The Server will delete this node. Its local identity is cleared after confirmation. An offline request remains pending.") }
        .sheet(isPresented: Binding(get: { selectedPeer != nil }, set: { if !$0 { selectedPeer = nil } })) {
            if let peer = model.status?.peers.first(where: { $0.nodeId == selectedPeer }) {
                PeerDetail(model: model, peer: peer)
            } else { Text("This peer is no longer in the installed netmap.").padding(32) }
        }
    }

    @ViewBuilder private var detail: some View {
        switch page ?? .overview {
        case .overview: overview
        case .peers: peers
        case .exit:
            if let selection = model.selection { ProfilesView(model: model, selection: selection) }
            else { ContentUnavailableView("No exit profiles", systemImage: "arrow.up.forward.circle", description: Text("Profiles appear when a netmap is installed.")) }
        case .diagnostics: DiagnosticsView(model: model)
        case .device: device
        }
    }

    private var overview: some View {
        Form {
            Section { ConnectionHeader(model: model, compact: false).padding(.vertical, 8) }
            Section("Network") {
                LabeledContent("Server", value: model.status?.server ?? model.server)
                AddressRow(title: "Mesh IPv4", value: model.status?.node?.ipv4, missing: model.status?.node == nil ? "Not installed" : "Not assigned")
                AddressRow(title: "Mesh IPv6", value: model.status?.node?.ipv6, missing: model.status?.node == nil ? "Not installed" : "Not assigned")
                LabeledContent("macOS VPN", value: model.vpnLabel)
            }
            Section("Connection health") {
                LabeledContent("Data plane", value: model.status.map { human($0.dataPlane) } ?? "Unavailable")
                LabeledContent("Control sync", value: model.status.map { human($0.sync) } ?? "Unavailable")
                LabeledContent("Last sync") { OptionalTime(date: model.status?.lastSyncAt) }
                LabeledContent("Lease expires") { OptionalTime(date: model.status?.netmap?.expiresAt) }
                LabeledContent("Netmap", value: model.status?.netmap == nil ? "Not installed" : (model.netmapApplied ? "Applied" : "Waiting to apply"))
            }
            Section {
                Button { page = .exit } label: {
                    HStack { Label("Exit profile", systemImage: "arrow.up.forward.circle"); Spacer(); Text(model.selection.map { $0.appliedProfile + (model.netmapApplied ? "" : " · pending") } ?? "Unavailable").foregroundStyle(.secondary); Image(systemName: "chevron.right").font(.caption).foregroundStyle(.tertiary) }
                }.buttonStyle(.plain)
                Button { page = .peers } label: {
                    HStack { Label("Peers", systemImage: "desktopcomputer"); Spacer(); Text(model.status.map { String($0.peers.count) } ?? "Unavailable").foregroundStyle(.secondary).contentTransition(.numericText()); Image(systemName: "chevron.right").font(.caption).foregroundStyle(.tertiary) }
                }.buttonStyle(.plain)
            }
        }.formStyle(.grouped).lineLimit(1)
    }

    private var peers: some View {
        VStack(spacing: 0) {
            HStack {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Search name, address or Node ID", text: $model.search).textFieldStyle(.plain)
            }.padding(10).background(.background, in: RoundedRectangle(cornerRadius: 8)).padding(20)
            List {
                ForEach(model.peers, id: \.nodeId) { peer in
                    Button { selectedPeer = peer.nodeId } label: { PeerRow(model: model, peer: peer) }
                        .buttonStyle(.plain).contextMenu { PeerCopyActions(peer: peer) }
                        .listRowSeparator(.hidden)
                }
            }.listStyle(.inset).animation(reduceMotion ? nil : .smooth(duration: 0.22), value: model.peers.map(\.nodeId))
                .overlay {
                    if model.peers.isEmpty {
                        ContentUnavailableView(model.search.isEmpty ? "No visible peers" : "No matching peers", systemImage: "desktopcomputer", description: Text(model.search.isEmpty ? "Peers from the installed netmap appear here." : "Try another name or address."))
                    }
                }
        }
    }

    private var device: some View {
        Form {
            Section("Application icon") { IconPicker() }
            Section("This Mac") {
                LabeledContent("Name", value: model.status?.node?.hostname ?? "Not installed")
                AddressRow(title: "Mesh IPv4", value: model.status?.node?.ipv4, missing: model.status?.node == nil ? "Not installed" : "Not assigned")
                AddressRow(title: "Mesh IPv6", value: model.status?.node?.ipv6, missing: model.status?.node == nil ? "Not installed" : "Not assigned")
                LabeledContent("Key generation", value: model.status?.identity.map { String($0.keyGeneration) } ?? "No identity")
                LabeledContent("Data plane observed") { OptionalTime(date: model.status?.observedAt) }
                LabeledContent("Network changed") { OptionalTime(date: model.status?.lastNetworkChange) }
            }
            Section("System integration") {
                LabeledContent("Network Extension", value: model.sampleMode ? "Not inspected in preview" : model.tunnel.extensionState)
                LabeledContent("macOS VPN", value: model.vpnLabel)
                Button("Enable extension…") { model.tunnel.activate() }.disabled(model.sampleMode).help(unavailableHelp(model, fallback: "Request macOS Network Extension activation"))
                Text(model.tunnel.message).font(.caption).foregroundStyle(.secondary).lineLimit(2).frame(height: 32, alignment: .topLeading)
            }
            Section("Identity") {
                Button("Rotate key", systemImage: "key.horizontal") { Task { await model.identityOperation(logout: false) } }
                    .disabled(!model.canRotateKey || model.busy || model.sampleMode)
                    .help(unavailableHelp(model, fallback: model.canRotateKey ? "Rotate this node's key" : "Requires an enrolled identity that is not leaving"))
                Button("Log out of network…", role: .destructive) { showLogout = true }
                    .disabled(!model.canLogout || model.busy || model.sampleMode)
                    .help(unavailableHelp(model, fallback: model.canLogout ? "Permanently remove this node" : "No enrolled identity"))
            }
            Section("Installed configuration") {
                LabeledContent("Netmap", value: model.status?.netmap.map { "Revision \($0.reference.revision) · serial \($0.reference.serial)" } ?? "No live lease")
                LabeledContent("Missing rule sets", value: model.status?.netmap.map { String($0.missingArtifacts.count) } ?? "Not applicable")
                LabeledContent("Sync issue", value: model.status?.syncError.map(human) ?? "None reported")
                LabeledContent("Next retry") { OptionalTime(date: model.status?.retryAt) }
            }
        }.formStyle(.grouped).lineLimit(1)
    }
}

struct EnrollmentView: View {
    @Bindable var model: AppModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var advanced = false
    private var approval: Bool { model.status?.phase == .awaitingApproval }
    private var disabled: Bool { model.busy || model.server.isEmpty || model.sampleMode }

    var body: some View {
        VStack(spacing: 22) {
            Image(systemName: approval ? "person.badge.key" : "network")
                .font(.system(size: 40, weight: .light)).foregroundStyle(.tint)
                .frame(width: 88, height: 88)
                .glassEffect(.regular, in: .rect(cornerRadius: 24))
                .contentTransition(reduceMotion ? .identity : .symbolEffect(.replace))
                .symbolEffect(.pulse, options: .repeating, isActive: approval && !reduceMotion)
            VStack(spacing: 8) {
                Text(approval ? "Approve this Mac" : "Your network, on this Mac").font(.title.weight(.semibold))
                Text(approval ? "Finish approval in your browser. Meshway will continue automatically." : "Connect to your devices through your Meshway network.")
                    .foregroundStyle(.secondary).multilineTextAlignment(.center).frame(height: 40)
            }
            ZStack(alignment: .top) {
                if approval, let status = model.status {
                    VStack(spacing: 20) {
                        HStack(spacing: 10) { ProgressView().controlSize(.small); Text("Waiting for approval").foregroundStyle(.secondary) }.frame(height: 32)
                        if let link = status.enrollmentUrl, let url = model.approvalURL(link) {
                            Button("Open approval page", systemImage: "arrow.up.right.square") { NSWorkspace.shared.open(url) }
                                .buttonStyle(.glassProminent).controlSize(.large).disabled(model.sampleMode)
                                .help(unavailableHelp(model, fallback: "Reopen the browser approval page"))
                        } else {
                            Text("No approval URL was provided. Ask the Server operator to configure its Console origin.").font(.callout).foregroundStyle(.secondary)
                        }
                        Button("Cancel connection") { Task { await model.connect(false) } }.disabled(model.busy || model.sampleMode)
                            .help(unavailableHelp(model, fallback: "Request Node Down"))
                    }.transition(.opacity)
                } else {
                    VStack(alignment: .leading, spacing: 12) {
                        Text("Server URL").font(.callout.weight(.medium))
                        TextField("https://mesh.example", text: $model.server).textFieldStyle(.roundedBorder).controlSize(.large)
                            .accessibilityLabel("Server URL")
                            .disabled(model.tunnel.hasConfiguration || model.status != nil)
                            .onSubmit { if !disabled { Task { await model.connect(true) } } }
                            .help("Use the Server origin supplied by your administrator")
                        DisclosureGroup("Advanced enrollment", isExpanded: $advanced) {
                            SecureField("Enrollment key (optional)", text: $model.enrollmentKey).textFieldStyle(.roundedBorder).padding(.top, 8)
                        }.font(.callout).frame(height: 62, alignment: .top)
                            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: advanced)
                        Button { Task { await model.connect(true) } } label: {
                            HStack(spacing: 8) {
                                ProgressView().controlSize(.small).opacity(model.busy ? 1 : 0).frame(width: 16)
                                Text("Connect to Meshway")
                                Color.clear.frame(width: 16, height: 16)
                            }.frame(maxWidth: .infinity)
                        }.buttonStyle(.glassProminent).controlSize(.large).disabled(disabled)
                            .help(unavailableHelp(model, fallback: model.server.isEmpty ? "Enter your Server URL first" : "Connect and open browser approval"))
                        Button("Enable Network Extension…") { model.tunnel.activate() }.buttonStyle(.link).disabled(model.sampleMode)
                            .help(unavailableHelp(model, fallback: "Approve Meshway in System Settings when prompted"))
                        Text(model.sampleMode ? "macOS will ask you to approve the extension and VPN." : "\(model.tunnel.extensionState) · \(model.tunnel.message)")
                            .font(.caption).foregroundStyle(.secondary).lineLimit(2).frame(height: 30, alignment: .topLeading)
                            .help(model.tunnel.message)
                    }.transition(.opacity)
                }
            }.frame(width: 340, height: 252, alignment: .top)
                .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: approval)
        }.frame(maxWidth: .infinity, maxHeight: .infinity).padding(24)
    }
}
