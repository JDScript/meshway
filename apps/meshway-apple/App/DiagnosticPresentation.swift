import Foundation

struct PingReading: Identifiable {
    let id: String
    let duration: UInt64?
    let replied: Bool
    let timedOut: Bool
    let setup: Bool
}

// Derived from retained events, never a second measurement or clock source.
struct PingPresentation {
    let events: [ClientPingEvent]
    var decision: ClientDecision? {
        events.reversed().compactMap { if case .decision(let decision) = $0 { decision } else { nil } }.first
    }
    var readings: [PingReading] {
        var readings: [PingReading] = []
        for event in events {
            let reading: PingReading
            switch event {
            case .echo(let echo):
                reading = PingReading(id: "echo:\(echo.sequence)", duration: echo.outcome == .reply ? echo.durationUs : nil,
                                      replied: echo.outcome == .reply, timedOut: echo.outcome == .timeout, setup: echo.setup)
            case .sample(let observation, let measurement, _, _, _, _, let probe):
                guard let probe, probe.outcome != .pending else { continue }
                reading = PingReading(id: observation + ":" + measurement, duration: probe.outcome == .success ? probe.durationUs : nil,
                                      replied: probe.outcome == .success, timedOut: probe.outcome == .timeout, setup: false)
            default: continue
            }
            if let index = readings.firstIndex(where: { $0.id == reading.id }) { readings[index] = reading }
            else { readings.append(reading) }
        }
        return readings
    }
    var durations: [Double] { readings.filter { !$0.setup }.compactMap(\.duration).map { Double($0) / 1000 }.sorted() }
    var median: Double? {
        let values = durations
        guard !values.isEmpty else { return nil }
        let middle = values.count / 2
        return values.count.isMultiple(of: 2) ? (values[middle - 1] + values[middle]) / 2 : values[middle]
    }
    var counts: (sent: UInt32, success: UInt32, timeout: UInt32)? {
        for event in events.reversed() {
            if case .completed(let sent, let success, let timeout, _) = event { return (sent, success, timeout) }
        }
        guard !readings.isEmpty else { return nil }
        return (UInt32(readings.count), UInt32(readings.filter(\.replied).count), UInt32(readings.filter(\.timedOut).count))
    }
    var failure: String? {
        if let action = decision?.action, case .denied(let reason) = action { return "策略拒绝：" + deniedReason(reason) }
        for event in events.reversed() {
            if case .failed(let code) = event {
                switch code {
                case .unsupported: return "macOS 版暂不支持直连外部目标的 ping"
                case .permissionDenied: return "没有探测此目标的权限"
                case .notRunning: return "请先连接 Meshway"
                case .targetNotFound: return "没有找到目标"
                case .timeout: return "探测超时"
                case .busy: return "已有探测正在进行，请稍后再试"
                case .stopped: return "探测已停止"
                case .protocol: return "探测协议不一致"
                case .failed: return "探测失败；未报告更详细的原因"
                }
            }
        }
        return nil
    }
}

func deniedReason(_ reason: ClientUnavailableReason) -> String {
    switch reason {
    case .exitUnavailable: "出口不可用"
    case .unsupportedFamily: "不支持此地址族"
    case .egressConflict: "出口规则冲突"
    case .unauthorized: "未获业务授权"
    case .noPath: "没有可用路径"
    case .routingCycle: "路由包含循环"
    case .routingLimit: "超出路由限制"
    case .missingDependency: "缺少所需配置或规则集"
    case .inactiveNode: "节点未激活"
    }
}

struct NetcheckFact: Identifiable {
    let id: String
    let title: String
    let value: String
}
struct NetcheckPresentation {
    let report: ClientNetcheck
    var facts: [NetcheckFact] {
        var values = [NetcheckFact(id: "server", title: "Server", value: (report.serverReachable.map { $0 ? "可达" : "不可达" } ?? "未测") + " · " + (report.serverRttUs.map(milliseconds) ?? "RTT 未测"))]
        for (name, available) in [("UDP", report.udp), ("IPv4", report.ipv4), ("IPv6", report.ipv6)] {
            values.append(NetcheckFact(id: name, title: name, value: report.unavailable == nil ? (available ? "可用" : "未检测到") : "未测"))
        }
        if report.sockets.isEmpty { values.append(NetcheckFact(id: "nat", title: "NAT 映射", value: "未测")) }
        for (index, socket) in report.sockets.enumerated() {
            let mapping: String
            switch socket.endpointMapping {
            case .dependent: mapping = "随目的地变化 · 难以直连"
            case .independent: mapping = "与目的地无关"
            case .unknown: mapping = "未测"
            }
            values.append(NetcheckFact(id: "nat:\(index)", title: "NAT · \(socket.boundAddress)", value: mapping))
        }
        let addresses = Array(Set(report.sockets.flatMap(\.mappings).filter(\.fresh).map(\.mapped))).sorted()
        values.append(NetcheckFact(id: "public", title: "公网映射", value: addresses.isEmpty ? "未测" : addresses.joined(separator: "、")))
        return values
    }
}
