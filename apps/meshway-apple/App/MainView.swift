import SwiftUI

enum AppPage: String, CaseIterable, Identifiable {
    case peers, exit, diagnostics
    var id: String { rawValue }
    var title: String {
        switch self {
        case .peers: "设备"
        case .exit: "出口"
        case .diagnostics: "诊断"
        }
    }
}

struct MainView: View {
    @Bindable var model: AppModel
    @Bindable private var navigation = AppNavigation.shared
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var needsEnrollment: Bool { model.status?.identity == nil || model.status?.phase == .awaitingApproval }
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
                                if let selection = model.selection { ProfilesView(model: model, selection: selection).padding(12) }
                                else { ContentUnavailableView("暂无出口方案", systemImage: "arrow.up.forward", description: Text("安装 netmap 后显示可用方案。")) }
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
                ToolbarItem(placement: .navigation) { StatusIsland(model: model, toolbar: true).frame(width: 266) }.sharedBackgroundVisibility(.hidden)
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
            Text(approval ? "批准这台 Mac" : "接入你的 Meshway 网络").font(.title3.weight(.semibold))
            Text(approval ? "在浏览器中完成批准，Meshway 会自动继续。" : "输入管理员提供的 Server URL。")
                .font(.callout).foregroundStyle(.secondary)
            ZStack(alignment: .top) {
                if approval, let status = model.status {
                    VStack(spacing: 18) {
                        HStack(spacing: 8) { ProgressView().controlSize(.small); Text("等待浏览器批准").foregroundStyle(.secondary) }
                        if let link = status.enrollmentUrl, let url = model.approvalURL(link) {
                            Button("打开批准页面", systemImage: "arrow.up.right.square") { NSWorkspace.shared.open(url) }
                                .buttonStyle(.glassProminent).disabled(model.sampleMode)
                                .help(unavailableHelp(model, fallback: "重新打开浏览器批准页面"))
                        } else { Text("Server 未提供批准链接，请联系管理员配置 Console 地址。").font(.caption).foregroundStyle(.secondary) }
                        Button("取消连接") { Task { await model.connect(false) } }.disabled(model.busy || model.sampleMode)
                            .help(unavailableHelp(model, fallback: "请求 Node Down"))
                    }.padding(.top, 16)
                } else {
                    VStack(alignment: .leading, spacing: 12) {
                        TextField("https://mesh.example", text: $model.server).textFieldStyle(.roundedBorder).controlSize(.large)
                            .accessibilityLabel("Server URL")
                            .disabled(model.tunnel.hasConfiguration || model.status != nil)
                            .onSubmit { if !disabled { Task { await model.connect(true) } } }
                            .help("使用管理员提供的 Server 地址；现有网络不支持在此切换")
                        DisclosureGroup("高级 · Enrollment key", isExpanded: $advanced) {
                            SecureField("Enrollment key（可选）", text: $model.enrollmentKey).textFieldStyle(.roundedBorder).padding(.top, 6)
                        }.font(.caption).frame(height: 54, alignment: .top)
                            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: advanced)
                        Button { Task { await model.connect(true) } } label: {
                            Text(model.busy ? "正在连接…" : "连接 Meshway").frame(maxWidth: .infinity)
                        }.buttonStyle(.glassProminent).controlSize(.large).disabled(disabled)
                            .help(unavailableHelp(model, fallback: model.server.isEmpty ? "请先输入 Server URL" : "连接并打开浏览器批准页面"))
                        Button("启用 Network Extension…") { model.tunnel.activate() }.buttonStyle(.link).disabled(model.sampleMode)
                            .help(unavailableHelp(model, fallback: "在系统设置中批准 Meshway 的网络扩展"))
                    }
                }
            }.frame(width: 320, height: 178, alignment: .top)
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
