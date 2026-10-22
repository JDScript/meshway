import SwiftUI

@main
struct MeshwayEntry {
    @MainActor static func main() {
        #if DEBUG
        // Before any lookup: Bundle and Locale read the language list once.
        if let index = CommandLine.arguments.firstIndex(of: "--preview-locale"), CommandLine.arguments.count > index + 1 {
            UserDefaults.standard.set([CommandLine.arguments[index + 1]], forKey: "AppleLanguages")
        }
        if CommandLine.arguments.contains("--preview") { PreviewMode.run(); return }
        if CommandLine.arguments.contains("--model-tests") { ModelTests.run(); return }
        if CommandLine.arguments.contains("--issue-codes") { ModelTests.exportCodes() }
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
                .frame(minWidth: 780, minHeight: 500)
                .task { model.startPolling() }
        }
        .defaultSize(width: 820, height: 520)
        .windowStyle(.hiddenTitleBar)
        .commands {
            #if DEBUG
            CommandMenu(Text(verbatim: "Debug")) {
                Button { model.tunnel.send("status") } label: { Text(verbatim: "Read IPC Status") }
                Button { model.tunnel.stop() } label: { Text(verbatim: "Stop tunnel") }
            }
            #endif
        }
        MenuBarExtra("Meshway", systemImage: "network") {
            MenuPanel(model: model).task { model.startPolling() }
        }.menuBarExtraStyle(.window)
        Settings {
            SettingsView(model: model).environment(IconPreferences.shared)
                .task { model.startPolling() }
        }
    }
}

final class MeshwayAppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        IconPreferences.shared.apply()
    }
}
