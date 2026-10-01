import MeshwayModel
import SwiftUI

public struct MeshwayGlyph: View {
  let size: CGFloat
  public init(size: CGFloat = 68) { self.size = size }
  public var body: some View {
    Image(systemName: "point.3.connected.trianglepath.dotted")
      .font(.system(size: size * 0.48, weight: .medium))
      .foregroundStyle(.white).frame(width: size, height: size)
      .background(
        LinearGradient(
          colors: [.indigo, .indigo.opacity(0.75)], startPoint: .topLeading,
          endPoint: .bottomTrailing),
        in: RoundedRectangle(cornerRadius: size * 0.25)
      )
      .accessibilityHidden(true)
  }
}

struct OnboardingView: View {
  @Bindable var model: AppModel
  var body: some View {
    OnboardingContentView(
      server: $model.controllerDraft, deviceName: $model.deviceName,
      status: model.status, message: model.enrollment.message ?? model.validationMessage,
      busy: model.enrollment.busy, hasIdentity: model.enrollment.hasIdentity,
      canRestart: model.enrollment.canRestart
    ) {
      if !model.enrollment.hasIdentity { model.saveController() }
      guard model.validationMessage == nil else { return }
      Task { await model.signIn() }
    } restart: {
      Task { await model.enrollment.restartExpiredAttempt() }
    }
  }
}

public struct OnboardingContentView: View {
  @Binding var server: String
  @Binding var deviceName: String
  let status: AppStatus
  let message: String?
  let busy: Bool
  let hasIdentity: Bool
  let canRestart: Bool
  let signIn: () -> Void
  let restart: () -> Void
  public init(
    server: Binding<String>, deviceName: Binding<String>, status: AppStatus = .notSignedIn,
    message: String? = nil, busy: Bool = false, hasIdentity: Bool = false, canRestart: Bool = false,
    signIn: @escaping () -> Void = {}, restart: @escaping () -> Void = {}
  ) {
    _server = server
    _deviceName = deviceName
    self.status = status
    self.message = message
    self.busy = busy
    self.hasIdentity = hasIdentity
    self.canRestart = canRestart
    self.signIn = signIn
    self.restart = restart
  }
  public var body: some View {
    GeometryReader { geometry in
      ScrollView { content.frame(maxWidth: .infinity, minHeight: geometry.size.height) }
        .background(Surface.canvas)
    }.tint(.indigo)
  }
  public var content: some View {
    OnboardingLayout(status: status, message: message) {
      TextField("https://console.example.com", text: $server).autocorrectionDisabled()
        #if os(iOS)
          .textInputAutocapitalization(.never).keyboardType(.URL)
        #endif
        .disabled(hasIdentity).accessibilityLabel("Server URL")
    } nameField: {
      TextField("Device name", text: $deviceName).disabled(hasIdentity).accessibilityLabel(
        "Device name")
    } action: {
      if status != .revoked {
        Button(action: signIn) { SignInButtonLabel(status: status, busy: busy) }
          .buttonStyle(.plain).disabled(
            busy || server.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        if canRestart {
          Button("Restart expired sign-in", action: restart).buttonStyle(.borderless).disabled(busy)
        }
      }
    }.textFieldStyle(.plain)
  }
}

public struct OnboardingLayout<ServerField: View, NameField: View, Action: View>: View {
  let status: AppStatus
  let message: String?
  @ViewBuilder let serverField: () -> ServerField
  @ViewBuilder let nameField: () -> NameField
  @ViewBuilder let action: () -> Action
  public init(
    status: AppStatus = .notSignedIn, message: String? = nil,
    @ViewBuilder serverField: @escaping () -> ServerField,
    @ViewBuilder nameField: @escaping () -> NameField, @ViewBuilder action: @escaping () -> Action
  ) {
    self.status = status
    self.message = message
    self.serverField = serverField
    self.nameField = nameField
    self.action = action
  }
  public var body: some View {
    VStack(spacing: LayoutMetrics.sectionGap) {
      VStack(spacing: 16) {
        MeshwayGlyph()
        VStack(spacing: 8) {
          Text(status == .revoked ? "Device revoked" : "Your network.\nWithin reach.")
            .font(.largeTitle.weight(.bold)).multilineTextAlignment(.center)
          Text(
            status == .revoked
              ? "This device no longer has access to the controller."
              : "Bring this device into your Meshway network."
          )
          .font(.body).foregroundStyle(.secondary).multilineTextAlignment(.center)
        }
      }
      VStack(alignment: .leading, spacing: 16) {
        field("Server URL", content: serverField)
        field("Device name", content: nameField)
        if let message {
          Label(message, systemImage: "exclamationmark.circle")
            .font(.callout).foregroundStyle(status == .revoked ? Color.secondary : Color.orange)
            .fixedSize(horizontal: false, vertical: true)
        }
        action()
      }
      VStack(spacing: 12) {
        Text(
          status == .revoked
            ? "Reset this device from the network menu to sign in again.\nThis only removes its local identity."
            : "Your browser handles sign-in and approval.\nYour device identity stays in Keychain."
        )
        .font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.center)
        TunnelNotice()
      }
    }.frame(maxWidth: 360).padding(.horizontal, LayoutMetrics.inset)
      .padding(.vertical, LayoutMetrics.sectionGap)
  }
  private func field<Content: View>(_ title: String, @ViewBuilder content: () -> Content)
    -> some View
  {
    VStack(alignment: .leading, spacing: LayoutMetrics.groupGap) {
      Text(title).font(.caption.weight(.medium)).foregroundStyle(.secondary)
      content().frame(maxWidth: .infinity, alignment: .leading).padding(12)
        .background(Surface.card, in: RoundedRectangle(cornerRadius: 10))
        .overlay { RoundedRectangle(cornerRadius: 10).strokeBorder(.primary.opacity(0.08)) }
    }
  }
}

public struct SignInButtonLabel: View {
  let status: AppStatus
  let busy: Bool
  public init(status: AppStatus = .notSignedIn, busy: Bool = false) {
    self.status = status
    self.busy = busy
  }
  public var body: some View {
    HStack(spacing: 8) {
      if busy { ActivityIndicator() } else { Image(systemName: "arrow.up.right") }
      Text(
        busy ? status.title : status == .redeeming ? "Retry saved exchange" : "Sign in with browser"
      )
      .fontWeight(.semibold)
    }.foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 14)
      .background(Color.indigo, in: RoundedRectangle(cornerRadius: 10)).contentShape(Rectangle())
  }
}
