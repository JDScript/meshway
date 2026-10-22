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

    /// Feed real codec frames through the request's stream checks, in order.
    @MainActor static func load(_ name: String, into model: AppModel) async throws {
        let id = UUID().uuidString.lowercased()
        let sample: ClientDiagnosticSample
        let command: ClientCommand
        switch name {
        case "ping": sample = .ping; command = .ping(target: "studio-nas", count: 4)
        case "ping-exit": sample = .pingExit; command = .ping(target: "intranet.example", count: 4)
        case "ping-denied": sample = .pingDenied; command = .ping(target: "office-mac", count: 4)
        case "ping-unsupported": sample = .pingUnsupported; command = .ping(target: "203.0.113.9", count: 4)
        case "netcheck": sample = .netcheck; command = .netcheck
        default: throw IPCFailure.result
        }
        let request = try ClientRequest(requestId: id, command: command)
        for frame in try await clientSampleDiagnosticFrames(sample: sample, requestId: id) {
            switch try await request.decode(frame: frame) {
            case .ping(_, let event): model.pingEvents.append(event)
            case .netcheck(let report): model.netcheck = report
            default: throw IPCFailure.result
            }
        }
        if case .ping(let target, _) = command { model.pingTarget = target }
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
                case "connecting-auth": sample = .connectingAuthentication
                case "connecting-sync": sample = .connectingSync
                case "connecting-apply": sample = .connectingApply
                case "connecting-plane": sample = .connectingPlane
                case "no-peers": sample = .noPeers
                case "mesh-only": sample = .meshOnly
                case "server-cached": sample = .serverUnavailable
                case "lease-expired": sample = .leaseUnavailable
                default: throw IPCFailure.result
                }
                let model = try await model(sample)
                var diagnostics: [String] = []
                if let index = args.firstIndex(of: "--preview-diagnostics"), args.count > index + 1 {
                    diagnostics = args[index + 1].split(separator: ",").map(String.init)
                    for name in diagnostics { try await load(name, into: model) }
                }
                if let index = args.firstIndex(of: "--preview-search"), args.count > index + 1 { model.search = args[index + 1] }
                let dark = args.contains("--dark")
                NSApp.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                if let index = args.firstIndex(of: "--preview-section"), args.count > index + 1 {
                    guard AppPage(rawValue: args[index + 1]) != nil else { throw IPCFailure.result }
                    model.previewSection = args[index + 1]
                }
                let locale = Locale(identifier: Bundle.main.preferredLocalizations.first ?? "en")
                let main = window(MainView(model: model).environment(IconPreferences(defaults: nil)).environment(\.locale, locale), size: NSSize(width: 820, height: 520), title: "Meshway")
                let panel = window(MenuPanel(model: model).environment(\.locale, locale), size: NSSize(width: 316, height: 512), title: "Meshway menu panel", panel: true)
                let settings = window(SettingsView(model: model).environment(IconPreferences(defaults: nil)).environment(\.locale, locale), size: NSSize(width: 580, height: 510), title: "Meshway Settings")
                let popover = args.contains("--preview-popover")
                    ? window(IssuePopover(model: model).environment(\.locale, locale).background(.regularMaterial), size: NSSize(width: 340, height: 300), title: "Meshway details", panel: false)
                    : nil
                windows = [main, panel, settings] + (popover.map { [$0] } ?? [])
                popover?.setFrameOrigin(NSPoint(x: 400, y: 600))
                main.setFrameOrigin(NSPoint(x: 60, y: 80)); panel.setFrameOrigin(NSPoint(x: 950, y: 160))
                main.makeKeyAndOrderFront(nil); panel.orderFront(nil); NSApp.activate()
                if let index = args.firstIndex(of: "--snapshot-dir"), args.count > index + 1 {
                    try await Task.sleep(for: .seconds(2))
                    let directory = URL(fileURLWithPath: args[index + 1], isDirectory: true)
                    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
                    let stem = sampleName + (model.previewSection.map { "-\($0)" } ?? "") + (args.contains("--preview-selected") ? "-selected" : "") + (args.contains("--preview-search") ? "-search" : "") + (diagnostics.isEmpty ? "" : "-" + diagnostics.joined(separator: "+")) + (args.contains("--preview-locale") ? "-" + (Bundle.main.preferredLocalizations.first ?? "en") : "") + (dark ? "-dark" : "-light")
                    try await snapshot(main, to: directory.appendingPathComponent("\(stem)-main.png"))
                    try await snapshot(panel, to: directory.appendingPathComponent("\(stem)-panel.png"))
                    let settingsPage: String
                    if let index = args.firstIndex(of: "--preview-settings"), args.count > index + 1 { settingsPage = "-" + args[index + 1] }
                    else { settingsPage = "" }
                    try await snapshot(settings, to: directory.appendingPathComponent("\(stem)-settings\(settingsPage).png"))
                    if let popover { try await snapshot(popover, to: directory.appendingPathComponent("\(stem)-popover.png")) }
                    print("Preview screenshots saved to \(directory.path)")
                    main.close(); panel.close(); settings.close(); popover?.close(); exit(0)
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
        window.titleVisibility = .hidden
        let host = NSHostingController(rootView: view)
        host.sizingOptions = []
        window.contentViewController = host
        window.setContentSize(size)
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
        guard capture.terminationStatus == 0 else { throw PreviewFailure.capture }
    }
}

private enum PreviewFailure: Error { case capture }

private final class PreviewWindow: NSWindow {
    override var canBecomeKey: Bool { true }
}
#endif
