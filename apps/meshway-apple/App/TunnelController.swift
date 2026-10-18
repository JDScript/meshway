import Foundation
import NetworkExtension
import Observation
import SystemExtensions

@MainActor @Observable
final class TunnelController: NSObject, OSSystemExtensionRequestDelegate {
    var message = "Shell only — Node runtime is not linked." {
        didSet { report?(message) }
    }
    var report: ((String) -> Void)?
    private var activationCompletion: ((Int32) -> Void)?
    var frames = ""
    var busy = false
    private var manager: NETunnelProviderManager?
    private var client: FrameClient?

    func activate(completion: ((Int32) -> Void)? = nil) {
        activationCompletion = completion
        do {
            let request = OSSystemExtensionRequest.activationRequest(
                forExtensionWithIdentifier: try ShellConfiguration.value("MeshwayExtensionID"), queue: .main)
            request.delegate = self
            OSSystemExtensionManager.shared.submitRequest(request)
            message = "Activation requested"
        } catch { message = error.localizedDescription; finishActivation(1) }
    }

    func deactivate() {
        do {
            let request = OSSystemExtensionRequest.deactivationRequest(
                forExtensionWithIdentifier: try ShellConfiguration.value("MeshwayExtensionID"), queue: .main)
            request.delegate = self
            OSSystemExtensionManager.shared.submitRequest(request)
        } catch { message = error.localizedDescription }
    }

    @discardableResult
    func start(defaultRoute: Bool, includeAllNetworks: Bool, excludeLocalNetworks: Bool = false) async -> Bool {
        guard !busy else { return false }
        busy = true
        defer { busy = false }
        do {
            let identifier = try ShellConfiguration.value("MeshwayExtensionID")
            let managers = try await NETunnelProviderManager.loadAllFromPreferences()
            let manager = managers.first { ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == identifier } ?? NETunnelProviderManager()
            let configuration = NETunnelProviderProtocol()
            configuration.providerBundleIdentifier = identifier
            configuration.serverAddress = "Meshway shell lab"
            configuration.providerConfiguration = ["labDefaultRoute": defaultRoute]
            configuration.includeAllNetworks = includeAllNetworks
            configuration.excludeLocalNetworks = excludeLocalNetworks
            manager.protocolConfiguration = configuration
            manager.localizedDescription = "Meshway Shell Lab"
            manager.isEnabled = true
            try await manager.saveToPreferences()
            try await manager.loadFromPreferences()
            self.manager = manager
            try manager.connection.startVPNTunnel()
            message = "Tunnel start requested (shell drops captured packets)"
            return true
        } catch { message = error.localizedDescription; return false }
    }

    func stop() {
        disconnectFrames()
        manager?.connection.stopVPNTunnel()
        message = "Tunnel stop requested"
    }

    func disconnectFrames() {
        client?.close()
        client = nil
    }

    @discardableResult
    func removeConfiguration() async -> Bool {
        do {
            let identifier = try ShellConfiguration.value("MeshwayExtensionID")
            for item in try await NETunnelProviderManager.loadAllFromPreferences()
            where (item.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == identifier {
                item.connection.stopVPNTunnel()
                try await item.removeFromPreferences()
            }
            message = "Shell VPN configuration removed"
            return true
        } catch { message = error.localizedDescription; return false }
    }

    func send(_ text: String) {
        do {
            if client == nil {
                client = try FrameClient(output: { [weak self] frame, acknowledge in
                    Task { @MainActor in
                        defer { acknowledge() }
                        guard let self else { return }
                        self.frames = String((self.frames + "\n" + String(decoding: frame, as: UTF8.self)).suffix(8192))
                    }
                }, disconnected: { [weak self] in
                    Task { @MainActor in self?.client = nil; self?.message = "XPC disconnected" }
                })
            }
            client?.send(Data(text.utf8)) { [weak self] accepted in
                Task { @MainActor in self?.message = accepted ? "Frame acknowledged" : "Frame rejected / busy" }
            }
        } catch { message = error.localizedDescription }
    }

    nonisolated func requestNeedsUserApproval(_ request: OSSystemExtensionRequest) {
        Task { @MainActor in
            message = "Approve Meshway in System Settings → General → Login Items & Extensions → Network Extensions"
            finishActivation(77)
        }
    }
    nonisolated func request(_ request: OSSystemExtensionRequest, didFinishWithResult result: OSSystemExtensionRequest.Result) {
        let text = result == .completed ? "Extension request completed" : "Extension request requires reboot"
        Task { @MainActor in message = text; finishActivation(result == .completed ? 0 : 78) }
    }
    nonisolated func request(_ request: OSSystemExtensionRequest, didFailWithError error: Error) {
        let text = error.localizedDescription
        Task { @MainActor in message = text; finishActivation(1) }
    }
    private func finishActivation(_ status: Int32) {
        let completion = activationCompletion
        activationCompletion = nil
        completion?(status)
    }
    nonisolated func request(_ request: OSSystemExtensionRequest, actionForReplacingExtension existing: OSSystemExtensionProperties, withExtension ext: OSSystemExtensionProperties) -> OSSystemExtensionRequest.ReplacementAction { .replace }
}
