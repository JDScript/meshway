import Foundation
import MeshwayCore
import Observation

public enum AppStatus: Equatable, Sendable {
  case notConfigured
  case notSignedIn

  public var title: String {
    switch self {
    case .notConfigured: "Set up your controller"
    case .notSignedIn: "Not signed in"
    }
  }
  public var symbol: String {
    switch self {
    case .notConfigured: "gearshape"
    case .notSignedIn: "person.crop.circle.badge.questionmark"
    }
  }
}

@MainActor @Observable public final class AppModel {
  public var controllerDraft: String
  public private(set) var controller: ControllerAddress?
  public private(set) var validationMessage: String?
  public private(set) var preferencesDiagnostic = "System VPN preferences have not been inspected."
  public let coreInfo: CoreBuildInfo
  public var status: AppStatus { controller == nil ? .notConfigured : .notSignedIn }
  public var authUnavailableReason: String { auth.unavailableReason }
  public var tunnelUnavailableReason: String { tunnel.unavailableReason }
  @ObservationIgnored private let auth: any AuthProvider
  @ObservationIgnored private let tunnel: any TunnelController
  @ObservationIgnored private let defaults: UserDefaults

  public init(
    core: any CoreService = RustCoreService(),
    auth: any AuthProvider = UnavailableOAuthProvider(),
    tunnel: any TunnelController, defaults: UserDefaults = .standard
  ) {
    self.auth = auth
    self.tunnel = tunnel
    self.defaults = defaults
    coreInfo = core.buildInfo()
    controllerDraft = defaults.string(forKey: "controllerURL") ?? ""
    controller = try? ControllerAddress(controllerDraft)
  }

  public func saveController() {
    do {
      let address = try ControllerAddress(controllerDraft)
      controller = address
      controllerDraft = address.url.absoluteString
      defaults.set(controllerDraft, forKey: "controllerURL")
      validationMessage = nil
    } catch {
      validationMessage = error.localizedDescription
    }
  }

  public func clearController() {
    defaults.removeObject(forKey: "controllerURL")
    controller = nil
    controllerDraft = ""
    validationMessage = nil
  }

  public func inspectPreferences() async {
    do {
      let count = try await tunnel.installedConfigurationCount()
      preferencesDiagnostic =
        "Installed Meshway VPN configurations: \(count). This does not indicate a working tunnel."
    } catch {
      preferencesDiagnostic = "Cannot read system VPN preferences: \(error.localizedDescription)"
    }
  }
}
