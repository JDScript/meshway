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
    var probeStatus: ClientStatus?
    var busy = false
    var vpnStatus: NEVPNStatus = .invalid
    var hasConfiguration = false
    var preferencesLoaded = false
    var preferencesError: String?
    var serverOrigin: String?
    var extensionState = "Not checked"
    private var propertiesRequest: OSSystemExtensionRequest?
    private var propertiesChecked = false
    private var manager: NETunnelProviderManager?
    private var frameTask: Task<Void, Never>?

    func refresh() async {
        do {
            let identifier = try ShellConfiguration.value("MeshwayExtensionID")
            if !propertiesChecked {
                propertiesChecked = true
                let request = OSSystemExtensionRequest.propertiesRequest(forExtensionWithIdentifier: identifier, queue: .main)
                request.delegate = self
                propertiesRequest = request
                OSSystemExtensionManager.shared.submitRequest(request)
            }
            manager = try await NETunnelProviderManager.loadAllFromPreferences().first {
                ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == identifier
            }
            hasConfiguration = manager != nil
            preferencesLoaded = true
            preferencesError = nil
            vpnStatus = manager?.connection.status ?? .invalid
            serverOrigin = (manager?.protocolConfiguration as? NETunnelProviderProtocol)?.providerConfiguration?["serverOrigin"] as? String
        } catch { preferencesError = error.localizedDescription; message = error.localizedDescription }
    }

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
    func start(defaultRoute: Bool, includeAllNetworks: Bool, excludeLocalNetworks: Bool = false, systemAuthorization: Bool = false, serverOrigin: String? = nil) async -> Bool {
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
            let origin = serverOrigin ?? ProcessInfo.processInfo.environment["MESHWAY_SERVER_URL"] ?? savedOrigin ?? "http://127.0.0.1:8430"
            configuration.serverAddress = origin
            let savedAddress = (manager.protocolConfiguration as? NETunnelProviderProtocol)?.providerConfiguration?["dnsAnchorAddress"] as? String
            var providerSettings = ["serverOrigin": origin]
            if let address = ProcessInfo.processInfo.environment["MESHWAY_DNS_ANCHOR_ADDRESS"] ?? savedAddress { providerSettings["dnsAnchorAddress"] = address }
            configuration.providerConfiguration = providerSettings
            configuration.includeAllNetworks = includeAllNetworks
            configuration.excludeLocalNetworks = excludeLocalNetworks
            manager.protocolConfiguration = configuration
            if manager.localizedDescription == nil { manager.localizedDescription = "Meshway" }
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
        frameTask?.cancel()
        frameTask = nil
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
        frameTask = Task {
            do {
                let exchange = IPCExchange()
                if case .status(let status) = try await exchange.run(.status, text: true) { probeStatus = status }
                frames = exchange.responseText ?? ""
                message = "IPC Status received"
            } catch { message = error.localizedDescription }
        }
    }

    nonisolated func requestNeedsUserApproval(_ request: OSSystemExtensionRequest) {
        Task { @MainActor in
            message = "Approve Meshway in System Settings → General → Login Items & Extensions → Network Extensions"
            extensionState = "Approval required"
            finishActivation(77)
        }
    }
    nonisolated func request(_ request: OSSystemExtensionRequest, didFinishWithResult result: OSSystemExtensionRequest.Result) {
        let text = result == .completed ? "Extension request completed" : "Extension request requires reboot"
        Task { @MainActor in message = text; extensionState = text; propertiesChecked = false; finishActivation(result == .completed ? 0 : 78) }
    }
    nonisolated func request(_ request: OSSystemExtensionRequest, didFailWithError error: Error) {
        let text = error.localizedDescription
        let identity = ObjectIdentifier(request)
        Task { @MainActor in
            if propertiesRequest.map(ObjectIdentifier.init) == identity { extensionState = "Unable to inspect extension: \(text)"; propertiesRequest = nil }
            else { message = text; extensionState = "Activation failed"; finishActivation(1) }
        }
    }
    nonisolated func request(_ request: OSSystemExtensionRequest, foundProperties properties: [OSSystemExtensionProperties]) {
        let state: String
        if properties.contains(where: { $0.isEnabled }) { state = "Enabled" }
        else if properties.contains(where: { $0.isAwaitingUserApproval }) { state = "Approval required" }
        else if properties.isEmpty { state = "Not installed" }
        else { state = "Installed, not enabled" }
        Task { @MainActor in extensionState = state; propertiesRequest = nil }
    }
    private func finishActivation(_ status: Int32) {
        let completion = activationCompletion
        activationCompletion = nil
        completion?(status)
    }
    nonisolated func request(_ request: OSSystemExtensionRequest, actionForReplacingExtension existing: OSSystemExtensionProperties, withExtension ext: OSSystemExtensionProperties) -> OSSystemExtensionRequest.ReplacementAction { .replace }
}
