import SwiftUI

enum SettingsPage: String, CaseIterable, Identifiable {
    case general, account, details, diagnostics
    var id: String { rawValue }
    var title: String {
        switch self {
        case .general: "通用"
        case .account: "账户与本机"
        case .details: "详细信息"
        case .diagnostics: "诊断"
        }
    }
    var symbol: String {
        switch self {
        case .general: "gearshape"
        case .account: "laptopcomputer"
        case .details: "info.circle"
        case .diagnostics: "waveform.path.ecg"
        }
    }
}

struct SettingsView: View {
    let model: AppModel
    @State private var page: SettingsPage = .general
    @State private var showLogout = false
    var body: some View {
        TabView(selection: $page) {
            general.tabItem { Label(SettingsPage.general.title, systemImage: SettingsPage.general.symbol) }.tag(SettingsPage.general)
            account.tabItem { Label(SettingsPage.account.title, systemImage: SettingsPage.account.symbol) }.tag(SettingsPage.account)
            details.tabItem { Label(SettingsPage.details.title, systemImage: SettingsPage.details.symbol) }.tag(SettingsPage.details)
            DiagnosticsView(model: model, netcheckOnly: true)
                .tabItem { Label(SettingsPage.diagnostics.title, systemImage: SettingsPage.diagnostics.symbol) }.tag(SettingsPage.diagnostics)
        }.frame(width: 580, height: 510)
            .overlay(alignment: .bottomTrailing) { if model.sampleMode { SampleBanner().padding(6) } }
            .confirmationDialog("永久注销此节点？", isPresented: $showLogout, titleVisibility: .visible) {
                Button("注销", role: .destructive) { Task { await model.identityOperation(logout: true) } }
                Button("取消", role: .cancel) {}
            } message: { Text("Server 将删除此节点，确认后清除本地身份。离线请求会保持待处理状态。") }
            .task {
                #if DEBUG
                if model.sampleMode, let index = CommandLine.arguments.firstIndex(of: "--preview-settings"), CommandLine.arguments.count > index + 1 {
                    page = SettingsPage(rawValue: CommandLine.arguments[index + 1]) ?? .general
                }
                #endif
            }
    }
    private var general: some View {
        Form { Section("应用图标") { IconPicker() } }.formStyle(.grouped)
    }
    private var account: some View {
        Form {
            Section("本机") {
                LabeledContent("名称", value: model.status?.node?.hostname ?? "尚未安装")
                AddressRow(title: "Mesh IPv4", value: model.status?.node?.ipv4, missing: model.status?.node == nil ? "尚未安装" : "未分配")
                AddressRow(title: "Mesh IPv6", value: model.status?.node?.ipv6, missing: model.status?.node == nil ? "尚未安装" : "未分配")
                LabeledContent("Server", value: model.status?.server ?? (model.server.isEmpty ? "未配置" : model.server))
            }
            Section("身份") {
                Button("轮换密钥", systemImage: "key.horizontal") { Task { await model.identityOperation(logout: false) } }
                    .disabled(!model.canRotateKey || model.busy || model.sampleMode)
                    .help(unavailableHelp(model, fallback: model.canRotateKey ? "轮换此节点的密钥" : "需要已接入且不处于注销中的身份"))
                Button("注销网络…", role: .destructive) { showLogout = true }
                    .disabled(!model.canLogout || model.busy || model.sampleMode)
                    .help(unavailableHelp(model, fallback: model.canLogout ? "永久删除此节点" : "尚无已接入身份"))
            }
            if let notice = model.notice { Text(notice).font(.caption).foregroundStyle(.secondary) }
        }.formStyle(.grouped)
    }
    private var details: some View {
        Form {
            Section("连接") {
                LabeledContent("Node", value: model.phase)
                LabeledContent("macOS VPN", value: model.vpnLabel)
                LabeledContent("Network Extension", value: model.sampleMode ? "预览不检查" : model.tunnel.extensionState)
                Button("启用 Network Extension…") { model.tunnel.activate() }.disabled(model.sampleMode)
                    .help(unavailableHelp(model, fallback: "请求 macOS 启用网络扩展"))
                if !model.sampleMode { Text(model.tunnel.message).font(.caption).foregroundStyle(.secondary) }
            }
            Section("状态") {
                LabeledContent("租约截止") { OptionalTime(date: model.status?.netmap?.expiresAt) }
                LabeledContent("Control sync", value: model.status.map { human($0.sync) } ?? "不可用")
                LabeledContent("上次同步") { OptionalTime(date: model.status?.lastSyncAt) }
                LabeledContent("数据面", value: model.status.map { human($0.dataPlane) } ?? "不可用")
                LabeledContent("数据面观测") { OptionalTime(date: model.status?.observedAt) }
                LabeledContent("网络变化") { OptionalTime(date: model.status?.lastNetworkChange) }
                LabeledContent("Key generation", value: model.status?.identity.map { String($0.keyGeneration) } ?? "尚无身份")
            }
            Section("已安装配置") {
                LabeledContent("Netmap", value: model.status?.netmap.map { "Revision \($0.reference.revision) · serial \($0.reference.serial)" } ?? "尚无有效租约")
                LabeledContent("安装", value: model.status?.netmap == nil ? "尚未安装" : (model.netmapApplied ? "已应用" : "等待应用"))
                LabeledContent("缺失规则集", value: model.status?.netmap.map { String($0.missingArtifacts.count) } ?? "不适用")
                LabeledContent("同步问题", value: model.status?.syncError.map(human) ?? "未报告")
                LabeledContent("下次重试") { OptionalTime(date: model.status?.retryAt) }
            }
            if let selection = model.selection {
                Section("出口选择") {
                    LabeledContent("Netmap 方案", value: selection.appliedProfile)
                    LabeledContent("请求", value: selection.requestedProfile ?? "跟随默认方案")
                    LabeledContent("默认", value: selection.defaultProfile)
                    LabeledContent("锁定", value: selection.locked ? "管理员已锁定" : "未锁定")
                    LabeledContent("请求可用性", value: selection.requestedUnavailable.map(human) ?? "未报告问题")
                    LabeledContent("自动方案", value: selection.auto?.profile ?? "不适用")
                    LabeledContent("自动依据", value: selection.auto.map { human($0.basis) } ?? "不适用")
                    LabeledContent("声明区域", value: selection.auto?.region ?? (selection.auto == nil ? "不适用" : "未声明"))
                }
            }
            if !ConnectionPresentation(model: model).errors.isEmpty {
                Section("问题") {
                    ForEach(Array(ConnectionPresentation(model: model).errors.enumerated()), id: \.offset) { _, error in Text(error).textSelection(.enabled) }
                }
            }
        }.formStyle(.grouped)
    }
}
