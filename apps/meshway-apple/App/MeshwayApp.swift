import SwiftUI

@main
struct MeshwayEntry {
    @MainActor static func main() {
        #if DEBUG
        if CommandLine.arguments.contains("--preview") { PreviewMode.run(); return }
        if CommandLine.arguments.contains("--model-tests") { ModelTests.run(); return }
        #endif
        if CommandLine.arguments.contains("--probe") { Probe.run() }
        else if CommandLine.arguments.contains("--lab") { LabCommand.run() }
        else { MeshwayApp.main() }
    }
}

struct MeshwayApp: App {
    @NSApplicationDelegateAdaptor(MeshwayAppDelegate.self) private var delegate
    @State private var model = AppModel()
    var body: some Scene {
        Window("Meshway", id: "main") {
            MainView(model: model)
                .environment(IconPreferences.shared)
                .frame(minWidth: 720, minHeight: 620)
                .task { model.startPolling() }
        }
        .defaultSize(width: 900, height: 720)
        .commands {
            #if DEBUG
            CommandMenu("Debug") {
                Button("Read IPC Status") { model.tunnel.send("status") }
                Button("Stop tunnel") { model.tunnel.stop() }
            }
            #endif
        }
        MenuBarExtra("Meshway", systemImage: "network") {
            MenuPanel(model: model).task { model.startPolling() }
        }.menuBarExtraStyle(.window)
    }
}

final class MeshwayAppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        IconPreferences.shared.apply()
    }
}
