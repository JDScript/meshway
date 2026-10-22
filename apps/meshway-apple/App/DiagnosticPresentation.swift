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
    var failure: AppIssue? {
        if let action = decision?.action, case .denied(let reason) = action { return AppIssue(.route(reason)) }
        for event in events.reversed() {
            if case .failed(let code) = event { return AppIssue(.ping(code)) }
        }
        return nil
    }
}

struct NetcheckFact: Identifiable {
    let id: String
    let title: String
    let value: String
    var help: String?
}
struct NetcheckPresentation {
    let report: ClientNetcheck
    var issue: AppIssue? { report.unavailable.map { AppIssue(.netcheck($0)) } }
    var facts: [NetcheckFact] {
        let notTested = String(localized: "Not tested")
        let reachable = report.serverReachable.map { $0 ? String(localized: "Reachable") : String(localized: "Unreachable") } ?? notTested
        var values = [NetcheckFact(id: "server", title: String(localized: "Server"), value: reachable + (report.serverRttUs.map { " · " + milliseconds($0) } ?? ""))]
        for (name, available) in [("UDP", report.udp), ("IPv4", report.ipv4), ("IPv6", report.ipv6)] {
            values.append(NetcheckFact(id: name, title: name, value: report.unavailable == nil ? (available ? String(localized: "Available") : String(localized: "Not detected")) : notTested))
        }
        if report.sockets.isEmpty { values.append(NetcheckFact(id: "nat", title: String(localized: "NAT mapping"), value: notTested)) }
        for (index, socket) in report.sockets.enumerated() {
            let mapping: String
            switch socket.endpointMapping {
            case .dependent: mapping = String(localized: "Varies by destination · direct links are harder")
            case .independent: mapping = String(localized: "Stable")
            case .unknown: mapping = notTested
            }
            values.append(NetcheckFact(id: "nat:\(index)", title: String(localized: "NAT mapping"), value: mapping, help: socket.boundAddress))
        }
        let addresses = Array(Set(report.sockets.flatMap(\.mappings).filter(\.fresh).map(\.mapped))).sorted()
        values.append(NetcheckFact(id: "public", title: String(localized: "Public address"), value: addresses.isEmpty ? notTested : addresses.formatted(.list(type: .and))))
        return values
    }
}
