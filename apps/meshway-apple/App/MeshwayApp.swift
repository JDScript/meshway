import SwiftUI
import Synchronization

@main
struct MeshwayEntry {
    @MainActor static func main() {
        if CommandLine.arguments.contains("--probe") {
            Probe.run()
        } else {
            MeshwayApp.main()
        }
    }
}

struct MeshwayApp: App {
    @State private var controller = TunnelController()
    var body: some Scene {
        Window("Meshway", id: "main") {
            DebugView(controller: controller).frame(minWidth: 540, minHeight: 480)
        }
        MenuBarExtra("Meshway", systemImage: "network") {
            MenuPanel(controller: controller)
        }
    }
}

private struct MenuPanel: View {
    let controller: TunnelController
    @Environment(\.openWindow) private var openWindow
    var body: some View {
        Text("Meshway — A2 shell")
        Button("Open Meshway") { openWindow(id: "main"); NSApp.activate() }
        Button("Stop tunnel") { controller.stop() }
        Divider()
        Button("Quit") { NSApp.terminate(nil) }
    }
}

private struct DebugView: View {
    let controller: TunnelController
    @State private var frame = "echo from Meshway"
    @State private var defaultRoute = false
    @State private var includeAllNetworks = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Meshway · System Extension Lab").font(.title2)
            Text("A2 shell: no identity, Control, DNS or packet forwarding.").foregroundStyle(.secondary)
            HStack {
                Button("Activate extension") { controller.activate() }
                Button("Start tunnel") { Task { await controller.start(defaultRoute: defaultRoute, includeAllNetworks: includeAllNetworks) } }.disabled(controller.busy)
                Button("Stop") { controller.stop() }
            }
            Toggle("Lab: capture default route (VM only)", isOn: $defaultRoute)
            Toggle("Lab: includeAllNetworks (VM only)", isOn: $includeAllNetworks)
            Text("Default capture: 10.203.0.0/24. Captured packets are dropped.").font(.caption)
            HStack {
                TextField("Opaque UTF-8 debug frame", text: $frame)
                Button("Send XPC frame") { controller.send(frame) }
            }
            Text(controller.message).textSelection(.enabled)
            ScrollView { Text(controller.frames).font(.system(.caption, design: .monospaced)).frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled) }
            HStack {
                Button("Remove VPN configuration") { Task { await controller.removeConfiguration() } }
                Button("Deactivate extension") { controller.deactivate() }
            }
        }.padding(24)
    }
}

private enum Probe {
    private struct Result {
        var echo = false
        var pushes = 0
        var acknowledged = false
        var finishing = false
    }
    // Run without NSApplication / GUI. A bounded 4-second observation includes
    // echo plus unsolicited pushes; closing the connection cancels the session.
    @MainActor static func run() {
        do {
            let result = Mutex(Result())
            let client = try FrameClient(output: { frame, acknowledge in
                defer { acknowledge() }
                result.withLock { state in
                    if frame == Data("meshway-xpc-probe".utf8) { state.echo = true }
                    if frame == Data("shell-fixture: runtime=absent".utf8) { state.pushes += 1 }
                }
                print("frame base64=\(frame.base64EncodedString())")
            }, disconnected: {
                if !result.withLock({ $0.finishing }) { print("XPC disconnected"); exit(2) }
            })
            client.send(Data("meshway-xpc-probe".utf8)) { accepted in
                result.withLock { $0.acknowledged = accepted }
                print("accepted=\(accepted)")
                if !accepted { exit(3) }
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 4) {
                let passed = result.withLock { state in
                    state.finishing = true
                    return state.echo && state.acknowledged && state.pushes >= 2
                }
                print(passed ? "PASS echo + continuous pushes" : "FAIL incomplete probe / timeout")
                client.close()
                exit(passed ? 0 : 4)
            }
            dispatchMain()
        } catch { print("probe error: \(error)"); exit(1) }
    }
}
