import Foundation
import NetworkExtension
import Observation
import SystemExtensions
import Security
import Darwin

@MainActor @Observable
final class TunnelController: NSObject, OSSystemExtensionRequestDelegate {
    var message = "Apple Host — experimental Mesh Node." {
        didSet { report?(message) }
    }
    var report: ((String) -> Void)?
    private var activationCompletion: ((Int32) -> Void)?
    var frames = ""
    var busy = false
    private var manager: NETunnelProviderManager?
    private var client: FrameClient?
    private var clientID: UUID?

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
    func start(defaultRoute: Bool, includeAllNetworks: Bool, excludeLocalNetworks: Bool = false, systemAuthorization: Bool = false) async -> Bool {
        guard !defaultRoute, !includeAllNetworks else { message = "Default capture lab switches are not enabled for this Mesh-only package"; return false }
        guard !busy else { return false }
        busy = true
        defer { busy = false }
        do {
            let identifier = try ShellConfiguration.value("MeshwayExtensionID")
            let managers = try await NETunnelProviderManager.loadAllFromPreferences()
            let manager = managers.first { ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == identifier } ?? NETunnelProviderManager()
            var authorization: AuthorizationRef?
            defer { if let authorization { AuthorizationFree(authorization, []) } }
            if systemAuthorization {
                guard getuid() == 0 else { message = "System configuration lab action requires root"; return false }
                let status = "system.preferences.network".withCString { rightName in
                    var item = AuthorizationItem(name: rightName, valueLength: 0, value: nil, flags: 0)
                    return withUnsafeMutablePointer(to: &item) { item in
                        var rights = AuthorizationRights(count: 1, items: item)
                        return AuthorizationCreate(&rights, nil, [.extendRights, .preAuthorize], &authorization)
                    }
                }
                guard status == errAuthorizationSuccess, let authorization else {
                    message = "Network configuration authorization failed (\(status))"; return false
                }
                manager.setAuthorization(authorization)
                message = "Network configuration authorization obtained"
            }
            let configuration = NETunnelProviderProtocol()
            configuration.providerBundleIdentifier = identifier
            let savedOrigin = (manager.protocolConfiguration as? NETunnelProviderProtocol)?.providerConfiguration?["serverOrigin"] as? String
            let origin = ProcessInfo.processInfo.environment["MESHWAY_SERVER_URL"] ?? savedOrigin ?? "http://127.0.0.1:8430"
            configuration.serverAddress = origin
            let savedAddress = (manager.protocolConfiguration as? NETunnelProviderProtocol)?.providerConfiguration?["dnsAnchorAddress"] as? String
            var providerSettings = ["serverOrigin": origin]
            if let address = ProcessInfo.processInfo.environment["MESHWAY_DNS_ANCHOR_ADDRESS"] ?? savedAddress { providerSettings["dnsAnchorAddress"] = address }
            configuration.providerConfiguration = providerSettings
            configuration.includeAllNetworks = includeAllNetworks
            configuration.excludeLocalNetworks = excludeLocalNetworks
            manager.protocolConfiguration = configuration
            manager.localizedDescription = "Meshway Shell Lab"
            manager.isEnabled = true
            try await manager.saveToPreferences()
            try await manager.loadFromPreferences()
            self.manager = manager
            try manager.connection.startVPNTunnel()
            message = "Apple Host Node start requested"
            return true
        } catch { message = error.localizedDescription; return false }
    }

    func stop() {
        disconnectFrames()
        manager?.connection.stopVPNTunnel()
        message = "Tunnel stop requested"
    }

    func disconnectFrames() {
        let previous = client
        client = nil
        clientID = nil
        previous?.close()
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
        disconnectFrames()
        do {
            if client == nil {
                let id = UUID()
                clientID = id
                client = try FrameClient(output: { [weak self] frame, acknowledge in
                    Task { @MainActor in
                        defer { acknowledge() }
                        guard let self, self.clientID == id else { return }
                        guard let body = try? Probe.body(frame) else { self.message = "Invalid IPC frame"; return }
                        self.frames = String((self.frames + "\n" + String(decoding: body, as: UTF8.self)).suffix(8192))
                    }
                }, disconnected: { [weak self] in
                    Task { @MainActor in
                        guard let self, self.clientID == id else { return }
                        self.client = nil; self.clientID = nil; self.message = "XPC disconnected"
                    }
                })
            }
            let (_, request) = try Probe.request(Probe.Command(type: "status"))
            let id = clientID
            client?.send(request) { [weak self] accepted in
                Task { @MainActor in
                    guard let self, self.clientID == id else { return }
                    self.message = accepted ? "Frame acknowledged" : "Frame rejected / busy"
                }
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
