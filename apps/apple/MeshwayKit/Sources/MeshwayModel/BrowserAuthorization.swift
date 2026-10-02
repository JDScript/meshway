import AuthenticationServices
import Foundation

#if os(macOS)
  import AppKit
#else
  import UIKit
#endif

public enum BrowserError: Error, LocalizedError, Sendable, Equatable {
  case cancelled, denied, invalidCallback, unavailable
  public var errorDescription: String? {
    switch self {
    case .cancelled: "Sign-in was cancelled. This pending enrollment can be resumed or removed."
    case .denied: "Device access was denied. This pending enrollment can be resumed or removed."
    case .invalidCallback: "The authorization callback did not match the saved attempt."
    case .unavailable: "The system browser session could not be opened."
    }
  }
}

public enum DeviceCallback: Equatable, Sendable {
  case code(String)
  case denied
  public static func validate(_ url: URL, state: String) throws -> DeviceCallback {
    guard let parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
      parts.scheme == "meshway", parts.host == "enroll", parts.percentEncodedPath == "/callback",
      parts.user == nil, parts.password == nil, parts.port == nil, parts.fragment == nil,
      let items = parts.queryItems,
      items.allSatisfy({ ["state", "code", "error"].contains($0.name) }),
      items.filter({ $0.name == "state" }).count == 1,
      items.first(where: { $0.name == "state" })?.value == state
    else { throw BrowserError.invalidCallback }
    let codes = items.filter { $0.name == "code" }
    let errors = items.filter { $0.name == "error" }
    if codes.count == 1, errors.isEmpty, let code = codes[0].value,
      code.count == 64, code.allSatisfy({ "0123456789abcdef".contains($0) })
    {
      return .code(code)
    }
    if errors.count == 1, codes.isEmpty, errors[0].value == "access_denied" { return .denied }
    throw BrowserError.invalidCallback
  }
}

@MainActor public protocol BrowserAuthorizing {
  func authorize(_ url: URL) async throws -> URL
  func cancel()
}

@MainActor
public final class SystemBrowserAuthorization: NSObject, BrowserAuthorizing,
  ASWebAuthenticationPresentationContextProviding
{
  private var session: ASWebAuthenticationSession?
  private var anchor: ASPresentationAnchor?
  public override init() { super.init() }
  public func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
    // Set before start(); authorize() refuses to start without a window.
    anchor ?? ASPresentationAnchor()
  }
  public func authorize(_ url: URL) async throws -> URL {
    guard session == nil else { throw BrowserError.unavailable }
    #if os(macOS)
      anchor =
        NSApplication.shared.keyWindow
        ?? NSApplication.shared.windows.first(where: { $0.isVisible })
    #else
      anchor = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        .filter { $0.activationState == .foregroundActive }.flatMap(\.windows).first {
          $0.isKeyWindow
        }
    #endif
    guard anchor != nil else { throw BrowserError.unavailable }
    defer {
      session = nil
      anchor = nil
    }
    return try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation { continuation in
        let session = ASWebAuthenticationSession(
          url: url, callbackURLScheme: "meshway",
          completionHandler: Self.completion(resuming: continuation))
        session.presentationContextProvider = self
        // Reuse the console's human-login cookie; App never reads that cookie.
        session.prefersEphemeralWebBrowserSession = false
        self.session = session
        if !session.start() { continuation.resume(throwing: BrowserError.unavailable) }
      }
    } onCancel: {
      Task { @MainActor in self.cancel() }
    }
  }
  public func cancel() { session?.cancel() }

  /// AuthenticationServices calls this on an XPC reply queue, not the main
  /// thread. It is built outside the actor so it cannot inherit main-actor
  /// isolation, which would trap at runtime under Swift 6 checks.
  nonisolated static func completion(
    resuming continuation: CheckedContinuation<URL, any Error>
  ) -> @Sendable (URL?, (any Error)?) -> Void {
    { callback, error in
      if let error = error as? ASWebAuthenticationSessionError, error.code == .canceledLogin {
        continuation.resume(throwing: BrowserError.cancelled)
      } else if error != nil || callback == nil {
        continuation.resume(throwing: BrowserError.unavailable)
      } else if let callback {
        continuation.resume(returning: callback)
      }
    }
  }
}
