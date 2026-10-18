import SwiftUI
import Synchronization

@main
struct MeshwayEntry {
    @MainActor static func main() {
        if CommandLine.arguments.contains("--probe") {
            Probe.run()
        } else if CommandLine.arguments.contains("--lab") {
            LabCommand.run()
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
        Text("Meshway — Node bridge lab")
        Button("Open Meshway") { openWindow(id: "main"); NSApp.activate() }
        Button("Stop tunnel") { controller.stop() }
        Divider()
        Button("Quit") { NSApp.terminate(nil) }
    }
}

private struct DebugView: View {
    let controller: TunnelController

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Meshway · System Extension Lab").font(.title2)
            Text("Experimental Mesh Node with local Mesh DNS. Runtime state is read through IPC.").foregroundStyle(.secondary)
            HStack {
                Button("Activate extension") { controller.activate() }
                Button("Start Node") { Task { await controller.start(defaultRoute: false, includeAllNetworks: false) } }.disabled(controller.busy)
                Button("Stop") { controller.stop() }
            }
            HStack {
                Button("Read IPC Status") { controller.send("status") }
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
