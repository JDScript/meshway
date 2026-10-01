#if DEBUG && os(macOS)
  import AppKit
  import MeshwayModel
  import MeshwayUI
  import SwiftUI

  // Development IPC only. The controller's admin authority stays in the external
  // script; this browser transports a real device-flow request and callback.
  @MainActor private final class SnapshotBrowser: BrowserAuthorizing {
    let directory: URL
    init(directory: URL) { self.directory = directory }
    func authorize(_ url: URL) async throws -> URL {
      try Data(url.absoluteString.utf8).write(
        to: directory.appendingPathComponent("authorization-request"), options: .atomic)
      for _ in 0..<300 {
        let callback = directory.appendingPathComponent("authorization-callback")
        if let data = try? Data(contentsOf: callback),
          let text = String(data: data, encoding: .utf8), let url = URL(string: text)
        {
          try FileManager.default.removeItem(at: callback)
          return url
        }
        try await Task.sleep(for: .milliseconds(100))
      }
      throw BrowserError.unavailable
    }
    func cancel() {}
  }

  @MainActor enum WindowSnapshots {
    private static let environment = ProcessInfo.processInfo.environment
    private static var started = false
    private static var panel: NSWindow?
    private static var directory: URL? {
      environment["MESHWAY_SNAPSHOT_DIR"].map { URL(fileURLWithPath: $0, isDirectory: true) }
    }
    static func makeModel() -> AppModel? {
      guard let directory,
        let service = environment["MESHWAY_SNAPSHOT_KEYCHAIN"],
        service.hasPrefix("dev.meshway.snapshot."),
        let server = environment["MESHWAY_SNAPSHOT_SERVER"]
      else { return nil }
      guard let defaults = UserDefaults(suiteName: service) else { return nil }
      defaults.set(server, forKey: "controllerURL")
      let model = AppModel(
        enrollment: EnrollmentCoordinator(
          browser: SnapshotBrowser(directory: directory),
          secrets: KeychainSecretStore(service: service),
          snapshots: FileEnrollmentSnapshotStore(
            url: directory.appendingPathComponent("enrollment.json"))),
        tunnel: SystemTunnelController(providerBundleID: "dev.meshway.snapshot.PacketTunnel"),
        defaults: defaults)
      model.deviceName = "Zeyu’s MacBook Pro"
      return model
    }
    static func run(model: AppModel) async {
      guard let directory,
        environment["MESHWAY_SNAPSHOT_KEYCHAIN"]?.hasPrefix("dev.meshway.snapshot.") == true,
        environment["MESHWAY_SNAPSHOT_SERVER"] != nil, !started
      else { return }
      started = true
      do {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        NSApp.appearance = NSAppearance(
          named: environment["MESHWAY_SNAPSHOT_APPEARANCE"] == "dark" ? .darkAqua : .aqua)
        try await Task.sleep(for: .seconds(1))
        guard
          let window = NSApp.windows.first(where: {
            $0.identifier?.rawValue.contains("main") == true
          })
            ?? NSApp.windows.first(where: { $0.title == "Meshway" })
        else { throw SnapshotError.window }
        let size: NSSize
        switch environment["MESHWAY_SNAPSHOT_SIZE"] {
        case "min": size = NSSize(width: 440, height: 520)
        case "wide": size = NSSize(width: 680, height: 680)
        default: size = NSSize(width: 500, height: 680)
        }
        window.setContentSize(size)
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        try Data().write(to: directory.appendingPathComponent("ready"))
        var lastID = ""
        while !Task.isCancelled {
          let commandURL = directory.appendingPathComponent("command.json")
          if let data = try? Data(contentsOf: commandURL),
            let command = try? JSONDecoder().decode(Command.self, from: data), command.id != lastID
          {
            lastID = command.id
            switch command.action {
            case "sign-in": await model.signIn()
            case "refresh": await model.enrollment.refresh()
            case "detail", "network", "dismiss", "search":
              NotificationCenter.default.post(
                name: Notification.Name("MeshwaySnapshotAction"), object: command.action)
            case "panel":
              let host = NSHostingView(rootView: MeshwayMenuView(model: model))
              let panel = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 360, height: 560),
                styleMask: [.borderless], backing: .buffered, defer: false)
              panel.contentView = host
              panel.setContentSize(host.fittingSize)
              panel.center()
              panel.orderFront(nil)
              self.panel = panel
            case "capture":
              for _ in 0..<300 {
                if !model.enrollment.refreshing { break }
                try await Task.sleep(for: .milliseconds(100))
              }
              if window.attachedSheet == nil && panel == nil {
                window.makeKeyAndOrderFront(nil)
                window.makeFirstResponder(nil)
                NSApp.activate(ignoringOtherApps: true)
              }
              try await Task.sleep(for: .milliseconds(700))
              try capture(window, to: directory.appendingPathComponent("\(command.id).png"))
              if let sheet = window.attachedSheet {
                try capture(sheet, to: directory.appendingPathComponent("\(command.id)-sheet.png"))
              }
              if let panel {
                try capture(panel, to: directory.appendingPathComponent("\(command.id)-panel.png"))
                panel.orderOut(nil)
                self.panel = nil
              }
            case "reset":
              model.enrollment.reset()
              if let service = environment["MESHWAY_SNAPSHOT_KEYCHAIN"] {
                UserDefaults.standard.removePersistentDomain(forName: service)
              }
            case "quit": NSApp.terminate(nil)
            default: throw SnapshotError.command
            }
            try await Task.sleep(for: .milliseconds(400))
            try Data(command.id.utf8).write(
              to: directory.appendingPathComponent("done"), options: .atomic)
          }
          try await Task.sleep(for: .milliseconds(100))
        }
      } catch {
        // Deliberately omit underlying errors, requests, and callback URLs.
        try? Data("Snapshot operation failed".utf8).write(
          to: directory.appendingPathComponent("failed"))
        NSApp.terminate(nil)
      }
    }
    private struct Command: Decodable {
      let id: String
      let action: String
    }
    private enum SnapshotError: Error { case window, bitmap, command }
    private static func capture(_ window: NSWindow, to url: URL) throws {
      guard let frame = window.contentView?.superview else { throw SnapshotError.window }
      frame.layoutSubtreeIfNeeded()
      window.displayIfNeeded()
      guard let bitmap = frame.bitmapImageRepForCachingDisplay(in: frame.bounds) else {
        throw SnapshotError.bitmap
      }
      frame.cacheDisplay(in: frame.bounds, to: bitmap)
      guard let png = bitmap.representation(using: .png, properties: [:]) else {
        throw SnapshotError.bitmap
      }
      try png.write(to: url, options: .atomic)
    }
  }
#endif
