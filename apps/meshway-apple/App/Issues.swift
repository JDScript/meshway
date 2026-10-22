import Foundation

// The App's only error and warning registry. A raw value is published once and
// never renamed; its lowercase slug names the public documentation page.
// Mappings below are the only path from IPC, system and transport facts to a code.
enum IssueCode: String, CaseIterable, Identifiable {
    // Server and identity
    case serverUnreachable = "ERROR_SERVER_UNREACHABLE"
    case serverBusy = "ERROR_SERVER_BUSY"
    case credentialRejected = "ERROR_CREDENTIAL_REJECTED"
    case enrollmentExpired = "ERROR_ENROLLMENT_EXPIRED"
    case enrollmentWithdrawn = "ERROR_ENROLLMENT_WITHDRAWN"
    case enrollmentCancelled = "ERROR_ENROLLMENT_CANCELLED"
    case protocolMismatch = "ERROR_PROTOCOL_MISMATCH"
    case configurationInvalid = "ERROR_CONFIGURATION_INVALID"
    case clockSkew = "ERROR_CLOCK_SKEW"
    case requestRejected = "ERROR_REQUEST_REJECTED"
    // This Mac
    case localStorage = "ERROR_LOCAL_STORAGE"
    case internalTask = "ERROR_INTERNAL_TASK"
    case modeUnsupported = "ERROR_MODE_UNSUPPORTED"
    case localNetwork = "ERROR_LOCAL_NETWORK"
    case tunnelInterface = "ERROR_TUNNEL_INTERFACE"
    case dnsSetup = "ERROR_DNS_SETUP"
    case transport = "ERROR_TRANSPORT"
    case settingsApply = "ERROR_SETTINGS_APPLY"
    case deviceBlocked = "ERROR_DEVICE_BLOCKED"
    case connectionRetrying = "ERROR_CONNECTION_RETRYING"
    case networkSetupFailed = "ERROR_NETWORK_SETUP_FAILED"
    case networkSuspended = "ERROR_NETWORK_SUSPENDED"
    // Background sync while this Mac keeps working offline
    case syncServerUnreachable = "WARN_SERVER_UNREACHABLE"
    case syncServerBusy = "WARN_SERVER_BUSY"
    case syncCredentialRejected = "WARN_CREDENTIAL_REJECTED"
    case syncSignatureInvalid = "WARN_CONFIG_SIGNATURE_INVALID"
    case syncServerKeyMismatch = "WARN_SERVER_KEY_MISMATCH"
    case syncAccessWithdrawn = "WARN_ACCESS_WITHDRAWN"
    case syncIdentityRejected = "WARN_IDENTITY_REJECTED"
    case syncProtocolMismatch = "WARN_PROTOCOL_MISMATCH"
    case syncLocalStorage = "WARN_LOCAL_STORAGE"
    // Rule sets
    case ruleSetMissing = "WARN_RULE_SET_MISSING"
    case ruleSetDownload = "WARN_RULE_SET_DOWNLOAD"
    case ruleSetIntegrity = "WARN_RULE_SET_INTEGRITY"
    case ruleSetStorage = "WARN_RULE_SET_STORAGE"
    case ruleSetLimit = "WARN_RULE_SET_LIMIT"
    // Exit profiles
    case profileNotAllowed = "WARN_PROFILE_NOT_ALLOWED"
    case profileLocked = "WARN_PROFILE_LOCKED"
    case profileConflict = "WARN_PROFILE_CONFLICT"
    // Requests to Meshway on this Mac
    case ipcVersionMismatch = "ERROR_IPC_VERSION_MISMATCH"
    case ipcDisconnected = "ERROR_IPC_DISCONNECTED"
    case ipcRejected = "ERROR_IPC_REJECTED"
    case ipcUnexpected = "ERROR_IPC_UNEXPECTED"
    case ipcTimeout = "WARN_IPC_TIMEOUT"
    case requestConflict = "WARN_REQUEST_CONFLICT"
    case nodeBusy = "WARN_NODE_BUSY"
    case nodeStopping = "WARN_NODE_STOPPING"
    case deviceNotReady = "WARN_DEVICE_NOT_READY"
    case operationPending = "WARN_OPERATION_PENDING"
    case deviceLeaving = "WARN_DEVICE_LEAVING"
    case unexpected = "ERROR_UNEXPECTED"
    // macOS VPN and extension
    case serverURLInvalid = "ERROR_SERVER_URL_INVALID"
    case vpnPreferences = "ERROR_VPN_PREFERENCES"
    case vpnStartFailed = "ERROR_VPN_START_FAILED"
    case vpnStartTimeout = "ERROR_VPN_START_TIMEOUT"
    case extensionActivationFailed = "ERROR_EXTENSION_ACTIVATION_FAILED"
    case extensionApprovalRequired = "WARN_EXTENSION_APPROVAL_REQUIRED"
    case extensionRebootRequired = "WARN_EXTENSION_REBOOT_REQUIRED"
    case extensionStatusUnavailable = "WARN_EXTENSION_STATUS_UNAVAILABLE"
    case approvalLinkMissing = "WARN_APPROVAL_LINK_MISSING"
    // Routes decided for a ping target
    case routeExitUnavailable = "ERROR_ROUTE_EXIT_UNAVAILABLE"
    case routeFamilyUnsupported = "ERROR_ROUTE_FAMILY_UNSUPPORTED"
    case routeExitConflict = "ERROR_ROUTE_EXIT_CONFLICT"
    case routeNotAllowed = "ERROR_ROUTE_NOT_ALLOWED"
    case routeNoPath = "ERROR_ROUTE_NO_PATH"
    case routeLoop = "ERROR_ROUTE_LOOP"
    case routeTooComplex = "ERROR_ROUTE_TOO_COMPLEX"
    case routeRulesMissing = "ERROR_ROUTE_RULES_MISSING"
    case routeDeviceInactive = "ERROR_ROUTE_DEVICE_INACTIVE"
    // Ping and network check
    case pingNotConnected = "ERROR_PING_NOT_CONNECTED"
    case pingUnsupported = "ERROR_PING_UNSUPPORTED"
    case pingPermissionDenied = "ERROR_PING_PERMISSION_DENIED"
    case pingTargetNotFound = "ERROR_PING_TARGET_NOT_FOUND"
    case pingFailed = "ERROR_PING_FAILED"
    case pingTimeout = "WARN_PING_TIMEOUT"
    case pingBusy = "WARN_PING_BUSY"
    case pingStopped = "WARN_PING_STOPPED"
    case netcheckNotConnected = "WARN_NETCHECK_NOT_CONNECTED"
    case netcheckBusy = "WARN_NETCHECK_BUSY"

    var id: String { rawValue }
    var isWarning: Bool { rawValue.hasPrefix("WARN_") }
    /// Documentation page name: `ERROR_SERVER_URL_INVALID` → `error-server-url-invalid`.
    var slug: String { rawValue.lowercased().replacingOccurrences(of: "_", with: "-") }
    /// One sentence for end users, from the String Catalog under the code itself.
    var message: String { Bundle.main.localizedString(forKey: rawValue, value: nil, table: nil) }

    static func problem(_ value: ClientProblem) -> Self {
        switch value {
        case .network: .serverUnreachable
        case .serverBusy: .serverBusy
        case .credentials: .credentialRejected
        case .enrollmentExpired: .enrollmentExpired
        case .enrollmentWithdrawn: .enrollmentWithdrawn
        case .enrollmentCancelled: .enrollmentCancelled
        case .storage: .localStorage
        case .contract: .protocolMismatch
        case .configuration: .configurationInvalid
        case .clock: .clockSkew
        case .worker: .internalTask
        case .unsupportedMode: .modeUnsupported
        case .underlay: .localNetwork
        case .tun: .tunnelInterface
        case .dns: .dnsSetup
        case .transport: .transport
        case .apply: .settingsApply
        }
    }
    static func sync(_ value: ClientSyncError) -> Self {
        switch value {
        case .network: .syncServerUnreachable
        case .serverBusy: .syncServerBusy
        case .credentials: .syncCredentialRejected
        case .signatureInvalid: .syncSignatureInvalid
        case .serverKeyMismatch: .syncServerKeyMismatch
        case .withdrawn: .syncAccessWithdrawn
        case .identityRejected: .syncIdentityRejected
        case .contract: .syncProtocolMismatch
        case .storage: .syncLocalStorage
        }
    }
    static func artifact(_ value: ClientArtifactProblem?) -> Self {
        switch value {
        case nil: .ruleSetMissing
        case .download: .ruleSetDownload
        case .integrity: .ruleSetIntegrity
        case .storage: .ruleSetStorage
        case .budget: .ruleSetLimit
        }
    }
    static func selection(_ value: ClientSelectionUnavailable) -> Self {
        switch value {
        case .notAllowed: .profileNotAllowed
        case .locked: .profileLocked
        }
    }
    static func ipc(_ value: ClientErrorCode) -> Self {
        switch value {
        case .version: .ipcVersionMismatch
        case .conflict, .requestConflict: .requestConflict
        case .busy: .nodeBusy
        case .stopping: .nodeStopping
        case .storage: .localStorage
        case .notReady: .deviceNotReady
        case .pending: .operationPending
        case .network: .serverUnreachable
        case .serverBusy: .serverBusy
        case .credentials: .credentialRejected
        case .contract: .protocolMismatch
        case .rejected: .requestRejected
        case .leaving: .deviceLeaving
        case .profileNotAllowed: .profileNotAllowed
        case .profileLocked: .profileLocked
        case .selectionConflict: .profileConflict
        }
    }
    static func route(_ value: ClientUnavailableReason) -> Self {
        switch value {
        case .exitUnavailable: .routeExitUnavailable
        case .unsupportedFamily: .routeFamilyUnsupported
        case .egressConflict: .routeExitConflict
        case .unauthorized: .routeNotAllowed
        case .noPath: .routeNoPath
        case .routingCycle: .routeLoop
        case .routingLimit: .routeTooComplex
        case .missingDependency: .routeRulesMissing
        case .inactiveNode: .routeDeviceInactive
        }
    }
    static func ping(_ value: ClientPingError) -> Self {
        switch value {
        case .notRunning: .pingNotConnected
        case .unsupported: .pingUnsupported
        case .permissionDenied: .pingPermissionDenied
        case .targetNotFound: .pingTargetNotFound
        case .timeout: .pingTimeout
        case .busy: .pingBusy
        case .stopped: .pingStopped
        case .protocol: .protocolMismatch
        case .failed: .pingFailed
        }
    }
    static func netcheck(_ value: ClientNetcheckUnavailable) -> Self {
        switch value {
        case .notRunning: .netcheckNotConnected
        case .busy: .netcheckBusy
        }
    }
    static func failure(_ value: IPCFailure) -> Self {
        switch value {
        case .disconnected: .ipcDisconnected
        case .rejected: .ipcRejected
        case .timeout: .ipcTimeout
        case .result: .ipcUnexpected
        case .remote(let code, _): ipc(code)
        }
    }
}

/// One visible error or warning. `detail` is untranslated diagnostic text
/// reported by the Node or macOS, shown beside the code, never as the message.
struct AppIssue: Identifiable, Equatable {
    let code: IssueCode
    var detail: String?
    var id: String { code.rawValue + (detail ?? "") }
    var message: String { code.message }
    init(_ code: IssueCode, detail: String? = nil) { self.code = code; self.detail = detail }
    init(error: Error) {
        switch error {
        case let failure as IPCFailure: self.init(.failure(failure), detail: failure.detail)
        case is ClientError: self.init(.ipcUnexpected, detail: error.localizedDescription)
        default: self.init(.unexpected, detail: error.localizedDescription)
        }
    }
}

/// Every mapped source value with its code: documentation export and the
/// registry completeness check both read this list. Mapping is the switches above.
enum IssueSources {
    static let problems: [ClientProblem] = [.network, .serverBusy, .credentials, .enrollmentExpired, .enrollmentWithdrawn, .enrollmentCancelled,
                                            .storage, .contract, .configuration, .clock, .worker, .unsupportedMode, .underlay, .tun, .dns, .transport, .apply]
    static let syncErrors: [ClientSyncError] = [.network, .serverBusy, .credentials, .signatureInvalid, .serverKeyMismatch, .withdrawn, .identityRejected, .contract, .storage]
    static let artifacts: [ClientArtifactProblem?] = [nil, .download, .integrity, .storage, .budget]
    static let selections: [ClientSelectionUnavailable] = [.notAllowed, .locked]
    static let ipcCodes: [ClientErrorCode] = [.version, .conflict, .requestConflict, .busy, .stopping, .storage, .notReady, .pending, .network,
                                              .serverBusy, .credentials, .contract, .rejected, .leaving, .profileNotAllowed, .profileLocked, .selectionConflict]
    static let routes: [ClientUnavailableReason] = [.exitUnavailable, .unsupportedFamily, .egressConflict, .unauthorized, .noPath, .routingCycle,
                                                    .routingLimit, .missingDependency, .inactiveNode]
    static let pings: [ClientPingError] = [.notRunning, .unsupported, .permissionDenied, .targetNotFound, .timeout, .busy, .stopped, .protocol, .failed]
    static let netchecks: [ClientNetcheckUnavailable] = [.notRunning, .busy]
    static let failures: [IPCFailure] = [.disconnected, .rejected, .timeout, .result]

    static var all: [(source: String, code: IssueCode)] {
        func name<T>(_ value: T) -> String { String(describing: value) }
        var values: [(String, IssueCode)] = []
        values += problems.map { ("Status.lastError / networkError.problem = \(name($0))", .problem($0)) }
        values += syncErrors.map { ("Status.syncError = \(name($0))", .sync($0)) }
        values += artifacts.map { ("Status.netmap.missingArtifacts, artifactProblem = \($0.map(name) ?? "null")", .artifact($0)) }
        values += selections.map { ("Status.netmap.routing.requestedUnavailable = \(name($0))", .selection($0)) }
        values += ipcCodes.map { ("IPC Error.code = \(name($0))", .ipc($0)) }
        values += routes.map { ("Ping decision action denied.reason = \(name($0))", .route($0)) }
        values += pings.map { ("Ping failed.code = \(name($0))", .ping($0)) }
        values += netchecks.map { ("Netcheck unavailable = \(name($0))", .netcheck($0)) }
        values += failures.map { ("App XPC request: \(name($0))", .failure($0)) }
        values += [
            ("Status.phase = blocked without lastError", .deviceBlocked),
            ("Status.phase = retrying without lastError", .connectionRetrying),
            ("Status.dataPlane = failed without lastError", .networkSetupFailed),
            ("Status.dataPlane = suspended without lastError", .networkSuspended),
            ("Bridge ClientError (frame, contract, completed, input)", .ipcUnexpected),
            ("Any other thrown error", .unexpected),
            ("clientServerOrigin rejected the Server URL", .serverURLInvalid),
            ("NETunnelProviderManager.loadAllFromPreferences failed", .vpnPreferences),
            ("TunnelController.start returned false", .vpnStartFailed),
            ("NEVPNStatus not connected 15 s after start", .vpnStartTimeout),
            ("OSSystemExtensionRequest activation failed", .extensionActivationFailed),
            ("OSSystemExtensionRequest needs user approval", .extensionApprovalRequired),
            ("OSSystemExtensionRequest result willCompleteAfterReboot", .extensionRebootRequired),
            ("OSSystemExtensionRequest properties request failed", .extensionStatusUnavailable),
            ("Status.phase = awaitingApproval without enrollmentUrl", .approvalLinkMissing),
        ]
        return values
    }
}

extension IPCFailure {
    var detail: String? { if case .remote(_, let message) = self { message } else { nil } }
}
