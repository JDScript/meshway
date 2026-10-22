import SwiftUI

enum AppPage: String, CaseIterable, Identifiable {
    case peers, exit, diagnostics
    var id: String { rawValue }
    var title: String {
        switch self {
        case .peers: String(localized: "Devices")
        case .exit: String(localized: "Exit")
        case .diagnostics: String(localized: "Diagnostics")
        }
    }
}

struct MainView: View {
    @Bindable var model: AppModel
    @Bindable private var navigation = AppNavigation.shared
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var needsEnrollment: Bool { model.status?.phase == .awaitingApproval || (model.status?.identity == nil && ConnectionPresentation(model: model).state != .connecting) }
    private var page: AppPage { navigation.page ?? .peers }

    var body: some View {
        NavigationStack {
            ZStack {
                Rectangle().fill(.regularMaterial).ignoresSafeArea()
                Group {
                    if needsEnrollment { EnrollmentView(model: model) }
                    else {
                        Group {
                            switch page {
                            case .peers: DevicesView(model: model)
                            case .exit:
                                if ConnectionPresentation(model: model).state == .connecting { SkeletonRows(count: 3).frame(maxHeight: .infinity, alignment: .top).padding(12) }
                                else if let selection = model.selection { ProfilesView(model: model, selection: selection).padding(12) }
                                else { EmptyStateView(kind: .profiles) }
                            case .diagnostics: DiagnosticsView(model: model)
                            }
                        }.id(page).transition(reduceMotion ? .opacity : .asymmetric(insertion: .move(edge: .trailing).combined(with: .opacity), removal: .opacity))
                    }
                }.padding(12)
            }
            .clipped()
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: page)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: needsEnrollment)
            .overlay(alignment: .bottomTrailing) {
                if model.sampleMode { SampleBanner().padding(7).allowsHitTesting(false) }
            }
            .toolbar {
                ToolbarItem(placement: .navigation) { StatusIsland(model: model, toolbar: true).frame(maxWidth: 280) }.sharedBackgroundVisibility(.hidden)
                ToolbarSpacer(.flexible, placement: .primaryAction)
                ToolbarItem(placement: .primaryAction) {
                    PagePicker(selection: Binding(get: { page }, set: { navigation.page = $0 }))
                }.sharedBackgroundVisibility(.hidden)
            }
            .toolbarBackground(.hidden, for: .windowToolbar)
        }
        .task {
            #if DEBUG
            if model.sampleMode, let section = model.previewSection { navigation.page = AppPage(rawValue: section) ?? .peers }
            #endif
        }
    }
}

struct PagePicker: View {
    @Binding var selection: AppPage
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        HStack(spacing: 2) {
            ForEach(AppPage.allCases) { page in
                Button { selection = page } label: {
                    Text(page.title).font(.system(size: 12, weight: .medium)).frame(width: 62, height: 28)
                        .foregroundStyle(selection == page ? Color.primary : Color.secondary)
                        .background(selection == page ? Color.primary.opacity(0.08) : .clear, in: Capsule())
                }.buttonStyle(.plain).accessibilityAddTraits(selection == page ? .isSelected : [])
            }
        }.padding(3).glassEffect(.regular, in: .capsule)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: selection)
    }
}

struct EnrollmentView: View {
    @Bindable var model: AppModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var advanced = false
    private var approval: Bool { model.status?.phase == .awaitingApproval }
    private var disabled: Bool { model.busy || model.server.isEmpty || model.sampleMode }

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: approval ? "person.badge.key" : "network")
                .font(.system(size: 30, weight: .light)).foregroundStyle(.teal)
                .frame(width: 64, height: 64).glassEffect(.regular, in: .rect(cornerRadius: 18))
            Text(approval ? "Approve this Mac" : "Join your Meshway network").font(.title3.weight(.semibold))
            Text(approval ? "Approve it in your browser. Meshway continues on its own." : "Enter the server address from your admin.")
                .font(.callout).foregroundStyle(.secondary)
            ZStack(alignment: .top) {
                if approval, let status = model.status {
                    VStack(spacing: 18) {
                        MeshLoader(label: String(localized: "Waiting for approval…"), labelVisible: true)
                        if let link = status.enrollmentUrl, let url = model.approvalURL(link) {
                            Button("Open approval page", systemImage: "arrow.up.right.square") { NSWorkspace.shared.open(url) }
                                .buttonStyle(.glassProminent).disabled(model.sampleMode)
                                .help(unavailableHelp(model, fallback: String(localized: "Open the approval page again")))
                        } else { IssueRow(issue: AppIssue(.approvalLinkMissing), compact: true) }
                        Button("Cancel") { Task { await model.connect(false) } }.disabled(model.busy || model.sampleMode)
                            .help(unavailableHelp(model, fallback: String(localized: "Stop connecting")))
                    }.padding(.top, 16)
                } else {
                    VStack(alignment: .leading, spacing: 12) {
                        TextField(text: $model.server) { Text(verbatim: "https://mesh.example") }.textFieldStyle(.roundedBorder).controlSize(.large)
                            .accessibilityLabel(Text("Server address"))
                            .disabled(model.tunnel.hasConfiguration || model.status != nil)
                            .onSubmit { if !disabled { Task { await model.connect(true) } } }
                            .help("Server address from your admin")
                        DisclosureGroup(isExpanded: $advanced) {
                            SecureField("Enrollment key (optional)", text: $model.enrollmentKey).textFieldStyle(.roundedBorder).padding(.top, 6)
                        } label: { Text("Advanced") }.font(.caption).frame(height: 54, alignment: .top)
                            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: advanced)
                        Button { Task { await model.connect(true) } } label: {
                            Text(model.busy ? "Connecting…" : "Connect").frame(maxWidth: .infinity)
                        }.buttonStyle(.glassProminent).controlSize(.large).disabled(disabled)
                            .help(unavailableHelp(model, fallback: model.server.isEmpty ? String(localized: "Enter a server address first") : String(localized: "Connect and approve in your browser")))
                        if let issue = model.issue { IssueRow(issue: issue, compact: true) }
                        Button("Enable network extension…") { model.tunnel.activate() }.buttonStyle(.link).disabled(model.sampleMode)
                            .help(unavailableHelp(model, fallback: String(localized: "Allow Meshway in System Settings")))
                    }
                }
            }.frame(width: 320, height: 178, alignment: .top)
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
