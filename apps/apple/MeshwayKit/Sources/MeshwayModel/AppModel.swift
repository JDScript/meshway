import Foundation
import MeshwayCore
import Observation

public enum AppStatus: Equatable, Sendable {
  case notConfigured
  case notSignedIn
  case awaitingBrowser, redeeming, enrolled, revoked, error

  public var title: String {
    switch self {
    case .notConfigured: "Set up your controller"
    case .notSignedIn: "Not signed in"
    case .awaitingBrowser: "Awaiting browser authorization"
    case .redeeming: "Redeeming authorization"
    case .enrolled: "Enrolled"
    case .revoked: "Device revoked"
    case .error: "Sign-in error"
    }
  }
  public var symbol: String {
    switch self {
    case .notConfigured: "gearshape"
    case .notSignedIn: "person.crop.circle.badge.questionmark"
    case .awaitingBrowser, .redeeming: "hourglass"
    case .enrolled: "person.crop.circle.badge.checkmark"
    case .revoked, .error: "exclamationmark.triangle"
    }
  }
}

@MainActor @Observable public final class AppModel {
  public var controllerDraft: String
  public private(set) var controller: ControllerAddress?
  public private(set) var validationMessage: String?
  public private(set) var preferencesDiagnostic = "System VPN preferences have not been inspected."
  public let coreInfo: CoreBuildInfo
  public let enrollment: EnrollmentCoordinator
  public var deviceName = ProcessInfo.processInfo.hostName
  public var status: AppStatus {
    if enrollment.snapshot?.phase == .revoked { return .revoked }
    if enrollment.message != nil { return .error }
    guard controller != nil else { return .notConfigured }
    switch enrollment.snapshot?.phase {
    case .awaitingBrowser: return .awaitingBrowser
    case .redeeming: return .redeeming
    case .enrolled: return .enrolled
    case .revoked: return .revoked
    default: return .notSignedIn
    }
  }
  public var tunnelUnavailableReason: String { tunnel.unavailableReason }
  @ObservationIgnored private let tunnel: any TunnelController
  @ObservationIgnored private let defaults: UserDefaults

  public init(
    core: any CoreService = RustCoreService(),
    enrollment: EnrollmentCoordinator = EnrollmentCoordinator(profiles: .system()),
    tunnel: any TunnelController, defaults: UserDefaults = .standard
  ) {
    self.enrollment = enrollment
    self.tunnel = tunnel
    self.defaults = defaults
    coreInfo = core.buildInfo()
    controllerDraft =
      enrollment.snapshot?.server
      ?? enrollment.profiles.first { $0.id == enrollment.activeProfileID }?.server
      ?? defaults.string(forKey: "controllerURL") ?? ""
    controller = try? ControllerAddress(controllerDraft)
  }

  public func saveController() {
    guard !enrollment.hasIdentity else {
      validationMessage = "Use Add network… to sign in to another server."
      return
    }
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
    guard !enrollment.hasIdentity else {
      validationMessage = "Remove this enrollment explicitly before clearing its server."
      return
    }
    defaults.removeObject(forKey: "controllerURL")
    controller = nil
    controllerDraft = ""
    validationMessage = nil
  }

  public func signIn() async {
    guard let controller else { return }
    await enrollment.signIn(
      server: controller.url.absoluteString, name: deviceName, version: coreInfo.version)
  }

  public func switchNetwork(_ id: String) async {
    await enrollment.switchProfile(id)
    if let server = enrollment.snapshot?.server
      ?? enrollment.profiles.first(where: { $0.id == id })?.server
    {
      controllerDraft = server
      controller = try? ControllerAddress(server)
    }
    validationMessage = nil
    await enrollment.foregroundContact()
  }

  public func addNetwork() async {
    let server = enrollment.snapshot?.server ?? controllerDraft
    await enrollment.addNetwork()
    controllerDraft = server
    controller = try? ControllerAddress(server)
    validationMessage = nil
  }

  public func removeNetwork() async {
    await enrollment.removeActiveProfile()
    if let server = enrollment.snapshot?.server {
      controllerDraft = server
      controller = try? ControllerAddress(server)
    }
    validationMessage = nil
    await enrollment.foregroundContact()
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
