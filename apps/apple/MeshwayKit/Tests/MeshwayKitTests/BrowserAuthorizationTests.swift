import AuthenticationServices
import Foundation
import Testing

@testable import MeshwayModel

/// The system invokes the session completion on an XPC reply queue. Calling it
/// off the main thread must resume the continuation instead of trapping.
@Suite struct BrowserCompletionTests {
  private func complete(_ callback: URL?, _ error: (any Error)?) async throws -> URL {
    try await withCheckedThrowingContinuation { continuation in
      let completion = SystemBrowserAuthorization.completion(resuming: continuation)
      DispatchQueue.global().async { completion(callback, error) }
    }
  }

  @Test func callbackFromBackgroundQueueResumes() async throws {
    let url = try #require(URL(string: "meshway://enroll/callback?state=s&error=access_denied"))
    #expect(try await complete(url, nil) == url)
  }

  @Test func cancellationFromBackgroundQueueMapsToCancelled() async {
    let error = ASWebAuthenticationSessionError(.canceledLogin)
    await #expect(throws: BrowserError.cancelled) { try await complete(nil, error) }
  }

  @Test func missingCallbackFromBackgroundQueueIsUnavailable() async {
    await #expect(throws: BrowserError.unavailable) { try await complete(nil, nil) }
  }
}
