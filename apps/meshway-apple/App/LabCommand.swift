import AppKit
import Darwin
import Foundation
import NetworkExtension

// Headless VM acceptance uses the same controller as the debug window.
// Exit 77 means GUI approval, 78 a reboot, 124 an unanswered system request.
enum LabCommand {
    @MainActor static func run() {
        var virtualized: Int32 = 0
        var size = MemoryLayout.size(ofValue: virtualized)
        guard sysctlbyname("kern.hv_vmm_present", &virtualized, &size, nil, 0) == 0,
              virtualized == 1,
              let option = CommandLine.arguments.firstIndex(of: "--lab"),
              CommandLine.arguments.count == option + 2
        else { print("--lab requires a macOS VM and one action"); exit(64) }
        let action = CommandLine.arguments[option + 1]
        let controller = TunnelController()
        controller.report = { print($0) }
        _ = NSApplication.shared
        NSApp.setActivationPolicy(.accessory)
        Task { @MainActor in
            switch action {
            case "activate":
                controller.activate { exit($0) }
            case "start-private", "start-system", "start-default", "start-all", "start-default-local", "start-all-local":
                let success = await controller.start(
                    defaultRoute: !["start-private", "start-system"].contains(action), includeAllNetworks: action.hasPrefix("start-all"),
                    systemAuthorization: action == "start-system")
                exit(success ? 0 : 1)
            case "inspect-preferences":
                do {
                    let identifier = try ShellConfiguration.value("MeshwayExtensionID")
                    let managers = try await NETunnelProviderManager.loadAllFromPreferences()
                    guard let manager = managers.first(where: {
                        ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == identifier
                    }), let configuration = manager.protocolConfiguration as? NETunnelProviderProtocol else {
                        print("No saved Meshway VPN configuration"); exit(1)
                    }
                    print("providerBundleIdentifier=\(identifier)")
                    print("includeAllNetworks=\(configuration.includeAllNetworks)")
                    print("excludeLocalNetworks=\(configuration.excludeLocalNetworks)")
                    exit(0)
                } catch { print("Preferences read failed: \(error)"); exit(1) }
            case "app-frame":
                controller.send("meshway-app-frame")
                try? await Task.sleep(for: .seconds(4))
                print(controller.frames)
                let passed = controller.probeStatus?.networkMode == "tun"
                controller.disconnectFrames()
                print(passed ? "PASS App controller IPC 13 Status" : "FAIL App controller Status")
                exit(passed ? 0 : 1)
            case "remove":
                exit(await controller.removeConfiguration() ? 0 : 1)
            default:
                print("Actions: activate, start-private, start-system (root), inspect-preferences, start-default, start-all, start-default-local, start-all-local, app-frame, remove")
                exit(64)
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 60) {
            print("Timed out waiting for the system request; check VM approval dialogs")
            exit(124)
        }
        withExtendedLifetime(controller) { NSApp.run() }
    }
}
