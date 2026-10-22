#if DEBUG
import Foundation
import NetworkExtension

// Executable model tests share the exact generated bindings linked into the App.
// --model-tests exits before NSApplication, XPC, preferences or Node startup.
enum ModelTests {
    @MainActor static func run() {
        Task { @MainActor in
            do {
                let model = try await PreviewMode.model(.running)
                precondition(model.status?.phase == .running)
                precondition(model.peers.count == 3)
                model.search = "100.64.0.4"
                precondition(model.peers.count == 1)
                precondition(model.peers.first?.rtt == nil, "Unmeasured RTT must remain nil")
                precondition(model.path(model.peers[0]) == "via meshway-tokyo")
                model.search = "HOME"
                precondition(model.path(model.peers[0]) == "Direct · idle")
                precondition(model.peerStateLabel(model.peers[0]) == "Direct · idle")
                precondition(model.peerPathDetail(model.peers[0]) == nil)
                precondition(model.canRequestSelection && model.canRotateKey && model.canLogout)
                precondition(model.selection?.requestedProfile == "work")
                precondition(model.selection?.appliedProfile == "work")
                precondition(model.netmapApplied)
                precondition(ConnectionPresentation(model: model).state == .connected)
                let pending = try await PreviewMode.model(.awaitingApproval)
                precondition(pending.phase == "Awaiting approval")
                precondition(ConnectionPresentation(model: pending).state == .approval)
                precondition(pending.status?.enrollmentUrl != nil)
                precondition(pending.sampleMode, "Sample approval must never open the browser")
                precondition(pending.approvalURL("file:///etc/passwd") == nil)
                let failed = try await PreviewMode.model(.error)
                precondition(failed.status?.lastError == .credentials)
                precondition(ConnectionPresentation(model: failed).state == .error)
                precondition(ConnectionPresentation(model: failed).errors.count == 2, "Control and network errors remain independent")
                precondition(failed.status?.networkError?.problem == .dns)
                precondition(failed.status?.node == nil && failed.peers.isEmpty)
                precondition(!failed.netmapApplied)
                precondition(!failed.canRequestSelection)
                let down = try await PreviewMode.model(.unenrolled)
                precondition(down.status?.node == nil)
                precondition(down.status?.lastSyncAt == nil)
                precondition(down.status?.netmap == nil)
                precondition(!down.canRequestSelection && !down.canRotateKey && !down.canLogout)
                precondition(ConnectionPresentation(model: down).state == .disconnected)
                for phase in [ClientPhase.starting, .synchronizing, .controlReady] {
                    precondition(IslandState.resolve(phase: phase, vpn: .connected, hasError: false, approvalRequired: false) == .connecting)
                }
                for phase in [ClientPhase.retrying, .blocked] {
                    precondition(IslandState.resolve(phase: phase, vpn: .connected, hasError: false, approvalRequired: false) == .error)
                }
                precondition(IslandState.resolve(phase: nil, vpn: .connected, hasError: false, approvalRequired: false) == .connecting, "NE connected alone does not mean Node running")
                precondition(IslandState.resolve(phase: nil, vpn: nil, hasError: false, approvalRequired: false) == .unknown)
                precondition(IslandState.resolve(phase: nil, vpn: .disconnected, hasError: false, approvalRequired: true) == .approval)
                precondition(IslandState.resolve(phase: .running, vpn: .connected, hasError: true, approvalRequired: false) == .error)
                precondition(IslandState.resolve(phase: .stopping, vpn: .connected, hasError: false, approvalRequired: false) == .stopping)
                precondition(IslandState.resolve(phase: .leaving, vpn: .connected, hasError: false, approvalRequired: false) == .leaving)
                print("PASS Swift model: codec fixtures, search, idle/relay paths, missing RTT, selection, approval, independent errors")
                exit(0)
            } catch { print("FAIL Swift model: \(error)"); exit(1) }
        }
        dispatchMain()
    }
}
#endif
