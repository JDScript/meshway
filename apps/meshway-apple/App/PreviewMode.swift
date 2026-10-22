#if DEBUG
import SwiftUI

// Deliberately bypasses poll/start/activation. Both windows use the production
// views and a real codec round-trip; no handwritten Swift protocol fixture.
enum PreviewMode {
    @MainActor private static var windows: [NSWindow] = []
    @MainActor static func model(_ sample: ClientSample) async throws -> AppModel {
        let model = AppModel()
        model.sampleMode = true
        let id = UUID().uuidString.lowercased()
        let request = try ClientRequest(requestId: id, command: .status)
        let frame = try await clientSampleFrame(sample: sample, requestId: id)
        guard case .status(let status) = try await request.decode(frame: frame) else { throw IPCFailure.result }
        model.accept(status)
        return model
    }

    @MainActor static func run() {
        _ = NSApplication.shared
        NSApp.setActivationPolicy(.regular)
        Task { @MainActor in
            do {
                let args = CommandLine.arguments
                guard let index = args.firstIndex(of: "--preview"), args.count > index + 1 else { throw IPCFailure.result }
                let sample: ClientSample
                let sampleName = args[index + 1]
                switch sampleName {
                case "unenrolled": sample = .unenrolled
                case "approval": sample = .awaitingApproval
                case "running": sample = .running
                case "error": sample = .error
                default: throw IPCFailure.result
                }
                let model = try await model(sample)
                let dark = args.contains("--dark")
                NSApp.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                if let index = args.firstIndex(of: "--preview-section"), args.count > index + 1 {
                    guard AppPage(rawValue: args[index + 1]) != nil else { throw IPCFailure.result }
                    model.previewSection = args[index + 1]
                }
                let main = window(MainView(model: model).environment(IconPreferences(defaults: nil)), size: NSSize(width: 900, height: 720), title: "Meshway · SAMPLE DATA")
                let panel = window(MenuPanel(model: model).clipShape(.rect(cornerRadius: 14)), size: NSSize(width: 344, height: 480), title: "Meshway menu panel · SAMPLE DATA", panel: true)
                windows = [main, panel]
                main.setFrameOrigin(NSPoint(x: 60, y: 80)); panel.setFrameOrigin(NSPoint(x: 950, y: 160))
                main.makeKeyAndOrderFront(nil); panel.orderFront(nil); NSApp.activate()
                if let index = args.firstIndex(of: "--snapshot-dir"), args.count > index + 1 {
                    try await Task.sleep(for: .seconds(2))
                    let directory = URL(fileURLWithPath: args[index + 1], isDirectory: true)
                    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
                    let stem = sampleName + (model.previewSection.map { "-\($0)" } ?? "") + (dark ? "-dark" : "-light")
                    try await snapshot(main, to: directory.appendingPathComponent("\(stem)-main.png"))
                    try await snapshot(panel, to: directory.appendingPathComponent("\(stem)-panel.png"))
                    print("Preview screenshots saved to \(directory.path)")
                    main.close(); panel.close(); exit(0)
                }
            } catch { print("Preview failed: \(error)"); exit(1) }
        }
        NSApp.run()
    }
    @MainActor private static func window<V: View>(_ view: V, size: NSSize, title: String, panel: Bool = false) -> NSWindow {
        let window = PreviewWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: panel ? [.borderless] : [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        if panel { window.isOpaque = false; window.backgroundColor = .clear }
        window.title = title
        window.contentView = NSHostingView(rootView: view)
        return window
    }
    @MainActor private static func snapshot(_ window: NSWindow, to url: URL) async throws {
        window.makeKeyAndOrderFront(nil)
        NSRunningApplication.current.activate(options: [])
        try await Task.sleep(for: .milliseconds(800))
        // Liquid Glass is composited by WindowServer. cacheDisplay produces black
        // selection masks and omits glass, so capture the actual window instead.
        let capture = Process()
        capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        capture.arguments = ["-x", "-o", "-l\(window.windowNumber)", url.path]
        try capture.run()
        capture.waitUntilExit()
        guard capture.terminationStatus == 0 else { throw IPCFailure.remote("Window capture failed; allow screen recording for the preview launcher.") }
    }
}

private final class PreviewWindow: NSWindow {
    override var canBecomeKey: Bool { true }
}
#endif
