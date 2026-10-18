#if DEBUG
import Foundation

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
                precondition(model.selection?.requestedProfile == "work")
                precondition(model.selection?.appliedProfile == "work")
                precondition(model.netmapApplied)
                let pending = try await PreviewMode.model(.awaitingApproval)
                precondition(pending.phase == "Awaiting approval")
                precondition(pending.status?.enrollmentUrl != nil)
                precondition(pending.sampleMode, "Sample approval must never open the browser")
                precondition(pending.approvalURL("file:///etc/passwd") == nil)
                let failed = try await PreviewMode.model(.error)
                precondition(failed.status?.lastError == .credentials)
                precondition(failed.status?.networkError?.problem == .dns)
                precondition(failed.status?.node == nil && failed.peers.isEmpty)
                precondition(!failed.netmapApplied)
                let down = try await PreviewMode.model(.unenrolled)
                precondition(down.status?.node == nil)
                precondition(down.status?.lastSyncAt == nil)
                precondition(down.status?.netmap == nil)
                print("PASS Swift model: codec fixtures, search, idle/relay paths, missing RTT, selection, approval, independent errors")
                exit(0)
            } catch { print("FAIL Swift model: \(error)"); exit(1) }
        }
        dispatchMain()
    }
}
#endif
