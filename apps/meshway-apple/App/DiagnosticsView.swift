import SwiftUI

struct DiagnosticsView: View {
    @Bindable var model: AppModel
    var netcheckOnly = false
    @State private var checking = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var ping: PingPresentation { PingPresentation(events: model.pingEvents) }
    var body: some View {
        VStack(spacing: 12) {
            if !netcheckOnly { pingCard }
            netcheckCard
            Spacer(minLength: 0)
        }.padding(4).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }
    private var pingCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass").foregroundStyle(.tertiary)
                TextField("设备名、IP 或域名", text: $model.pingTarget).textFieldStyle(.plain).onSubmit { model.ping() }
                Button(model.pingRunning ? "取消" : "Ping") {
                    if model.pingRunning { model.cancelPing() } else { model.ping() }
                }.buttonStyle(.glassProminent).tint(.teal)
                    .disabled(!model.pingRunning && (model.sampleMode || model.pingTarget.isEmpty))
                    .help(unavailableHelp(model, fallback: model.pingTarget.isEmpty ? "填写目标后开始探测" : "四次探测；可随时取消"))
            }.padding(8).background(.background.opacity(0.65), in: .rect(cornerRadius: 12))
            ZStack(alignment: .topLeading) {
                if model.pingEvents.isEmpty && !model.pingRunning {
                    VStack(alignment: .leading, spacing: 12) {
                        HStack(spacing: 8) {
                            ForEach(Array((model.status?.peers ?? []).prefix(3)), id: \.nodeId) { peer in
                                Button(peer.node.hostname) { model.pingTarget = peer.node.hostname }.buttonStyle(.glass).controlSize(.small)
                            }
                        }
                        Text("先显示目标按当前方案怎么走，再实测往返时间。")
                            .font(.system(size: 12)).foregroundStyle(.secondary)
                    }.padding(.top, 4)
                } else {
                    VStack(alignment: .leading, spacing: 7) {
                        if let decision = ping.decision { decisionRow(decision) }
                        else { Text("正在等待本地路径判定…").font(.caption).foregroundStyle(.secondary).frame(height: 26) }
                        if !ping.readings.isEmpty {
                            PingBars(readings: Array(ping.readings.suffix(24)))
                            HStack(spacing: 24) {
                                stat("中位", ping.median.map { String(format: "%.2f ms", $0) } ?? "未测")
                                stat("最低 / 最高", ping.durations.first.flatMap { low in ping.durations.last.map { String(format: "%.2f / %.2f ms", low, $0) } } ?? "未测")
                                stat("收到", ping.counts.map { "\($0.success) / \($0.sent)" } ?? "未测")
                                stat("丢包", ping.counts.flatMap { $0.sent > 0 ? String(format: "%.0f%%", 100 * Double($0.timeout) / Double($0.sent)) : nil } ?? "未测")
                                    .help("超时数 / 发送数；执行失败不计为丢包。未完成时为已观测样本。")
                            }
                            Text(ping.readings.contains(where: \.setup) ? "最近 24 个样本 · 浅色柱含建连；RTT 汇总不含建连样本" : "最近 24 个样本 · 空槽表示未收到成功的 RTT")
                                .font(.system(size: 10)).foregroundStyle(.tertiary)
                        } else { Text(model.pingRunning ? "正在测量…" : "尚无成功的测量").font(.caption).foregroundStyle(.secondary).padding(.top, 18) }
                    }
                }
            }.frame(height: 142, alignment: .top)
            Text(ping.failure ?? model.issue ?? " ").font(.system(size: 11)).foregroundStyle(.orange)
                .lineLimit(1).frame(height: 16).help(ping.failure ?? model.issue ?? "")
        }.padding(14).frame(maxWidth: .infinity).background(.background.opacity(0.5), in: .rect(cornerRadius: 18))
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: model.pingEvents.count)
    }
    private func stat(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(value).font(.system(size: 13, weight: .semibold)).contentTransition(reduceMotion ? .identity : .numericText())
            Text(title).font(.system(size: 11)).foregroundStyle(.secondary)
        }
    }
    private func decisionRow(_ decision: ClientDecision) -> some View {
        HStack(spacing: 8) {
            ScrollView(.horizontal) {
                HStack(spacing: 5) {
                    ForEach(Array(path(decision).enumerated()), id: \.offset) { index, name in
                        if index > 0 { Image(systemName: "arrow.right").foregroundStyle(.tertiary) }
                        Text(name).padding(.horizontal, 8).padding(.vertical, 4).background(.quaternary, in: Capsule())
                    }
                }
            }.scrollIndicators(.hidden)
            Text(summary(decision)).foregroundStyle(.secondary).lineLimit(1).help(summary(decision))
                .frame(maxWidth: 270, alignment: .trailing)
        }.font(.system(size: 11)).frame(height: 26)
    }
    private func path(_ decision: ClientDecision) -> [String] {
        var result = decision.path.map { id in id == model.status?.identity?.nodeId ? "本机" : model.name(id) }
        // Empty path is unknown/unmanaged, not evidence of a direct circuit.
        if let destination = decision.destination, decision.node == nil { result.append(destination) }
        return result.isEmpty ? ["路径未报告"] : result
    }
    private func summary(_ decision: ClientDecision) -> String {
        var parts: [String] = []
        if let profile = decision.profile { parts.append("方案 " + profile) }
        if let rule = decision.rule { parts.append("规则 " + (rule.remark.isEmpty ? rule.pointer : rule.remark)) }
        let action: String?
        switch decision.action {
        case .mesh: action = "Mesh"
        case .exit: action = "出口"
        case .direct: action = "本机直连"
        case .unmanaged: action = "本机网络"
        case .denied(let reason): action = "拒绝：" + deniedReason(reason)
        case nil: action = nil
        }
        return parts.joined(separator: " · ") + (action.map { (parts.isEmpty ? "" : " → ") + $0 } ?? "")
    }
    private var netcheckCard: some View {
        Group {
            if let report = model.netcheck {
                VStack(alignment: .leading, spacing: 12) {
                    HStack {
                        Text("网络检查").fontWeight(.semibold)
                        RelativeTime(date: report.observedAt).font(.caption)
                        Spacer()
                        checkButton("重新检查")
                    }
                    ScrollView {
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 200), alignment: .leading)], alignment: .leading, spacing: 8) {
                            ForEach(NetcheckPresentation(report: report).facts) { fact in
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(fact.title).foregroundStyle(.secondary).lineLimit(1)
                                    Text(fact.value).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                                }.font(.system(size: 11)).padding(9).frame(maxWidth: .infinity, alignment: .leading)
                                    .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 10))
                            }
                        }
                    }
                }
            } else {
                QuietEmptyState(symbol: "waveform.path", title: "检查这台 Mac 的网络",
                                explanation: "测试 Server 可达性、UDP、IPv4 / IPv6 和 NAT 映射。",
                                compact: true, actionTitle: checking ? "正在检查…" : "开始检查", action: runCheck,
                                actionDisabled: model.busy || model.sampleMode || checking,
                                actionHelp: unavailableHelp(model, fallback: "开始网络检查"))
            }
        }.frame(height: 170).padding(14).frame(maxWidth: .infinity)
            .background(.background.opacity(0.5), in: .rect(cornerRadius: 18))
            .overlay(alignment: .topTrailing) { if checking { ProgressView().controlSize(.small).padding(20) } }
    }
    private func checkButton(_ title: String) -> some View {
        Button(checking ? "正在检查…" : title, action: runCheck).buttonStyle(.glass).controlSize(.small)
            .disabled(model.busy || model.sampleMode || checking).help(unavailableHelp(model, fallback: "重新检查网络"))
    }
    private func runCheck() {
        checking = true
        Task { await model.perform(.netcheck); checking = false }
    }
}

struct PingBars: View {
    let readings: [PingReading]
    var body: some View {
        let maximum = max(readings.compactMap(\.duration).max() ?? 1, 1)
        HStack(alignment: .bottom, spacing: 4) {
            ForEach(readings) { reading in
                ZStack(alignment: .bottom) {
                    RoundedRectangle(cornerRadius: 3).strokeBorder(.quaternary).frame(height: 42)
                    if let duration = reading.duration {
                        RoundedRectangle(cornerRadius: 3).fill(.teal.opacity(reading.setup ? 0.35 : 0.85))
                            .frame(height: max(2, 42 * Double(duration) / Double(maximum)))
                    }
                }.frame(width: 9).help(reading.duration.map { milliseconds($0) + (reading.setup ? " · 含建连" : "") } ?? "没有成功的 RTT 测量")
            }
            Spacer(minLength: 0)
        }.frame(height: 42).accessibilityLabel("最近样本 RTT，未收到回复的样本为空槽")
    }
}
