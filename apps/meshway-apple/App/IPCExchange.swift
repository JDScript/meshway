import Foundation

// One authenticated XPC connection per request, matching the Unix IPC contract.
// Rust owns framing, validation and typed results. ACK follows consumption.
@MainActor
final class IPCExchange {
    private var client: FrameClient?
    private var continuation: CheckedContinuation<ClientResult, Error>?
    private var timeout: Task<Void, Never>?
    private var consume: Task<Void, Never>?
    private var request: ClientRequest?
    private var event: ((ClientPingEvent) -> Void)?
    private(set) var responseText: String?

    func run(_ command: ClientCommand, event: ((ClientPingEvent) -> Void)? = nil, text: Bool = false) async throws -> ClientResult {
        let request = try ClientRequest(requestId: UUID().uuidString.lowercased(), command: command)
        let frame = try await request.frame()
        self.request = request
        self.event = event
        let seconds: Double
        switch command {
        case .rotateKey, .logout: seconds = 70
        case .selectProfile, .clearProfile: seconds = 30
        default: seconds = 15
        }
        return try await withTaskCancellationHandler {
            try Task.checkCancellation()
            return try await withCheckedThrowingContinuation { continuation in
                self.continuation = continuation
                do {
                    client = try FrameClient(output: { [weak self] frame, acknowledge in
                        Task { @MainActor in
                            guard let self, self.continuation != nil else { acknowledge(); return }
                            self.consume = Task { @MainActor in
                                do {
                                    let result = try await request.decode(frame: frame)
                                    guard self.continuation != nil else { acknowledge(); return }
                                    if text { self.responseText = try await clientResponseText(frame: frame) }
                                    if case .ping(_, let event) = result {
                                        self.event?(event)
                                        acknowledge()
                                        switch event {
                                        case .completed, .failed: self.finish(.success(result))
                                        default: self.armTimeout(seconds)
                                        }
                                    } else { acknowledge(); self.finish(.success(result)) }
                                } catch { acknowledge(); self.finish(.failure(error)) }
                            }
                        }
                    }, disconnected: { [weak self] in
                        Task { @MainActor in self?.finish(.failure(IPCFailure.disconnected)) }
                    })
                    client?.send(frame) { [weak self] accepted in
                        if !accepted { Task { @MainActor in self?.finish(.failure(IPCFailure.rejected)) } }
                    }
                    armTimeout(seconds)
                } catch { finish(.failure(error)) }
            }
        } onCancel: {
            Task { @MainActor in self.finish(.failure(CancellationError())) }
        }
    }

    private func armTimeout(_ seconds: Double) {
        timeout?.cancel()
        timeout = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(seconds)) } catch { return }
            self?.finish(.failure(IPCFailure.timeout))
        }
    }
    private func finish(_ result: Result<ClientResult, Error>) {
        guard let continuation else { return }
        self.continuation = nil
        timeout?.cancel(); timeout = nil
        let previous = client; client = nil
        continuation.resume(with: result)
        previous?.close()
        request = nil; event = nil
    }
}

enum IPCFailure: LocalizedError {
    case disconnected, rejected, timeout, result, remote(ClientErrorCode, String)
    var errorDescription: String? {
        switch self {
        case .disconnected: "Cannot reach the Node extension. Check the VPN and extension status."
        case .rejected: "The Node could not accept this request."
        case .timeout: "No response before the deadline. A submitted operation may still complete; refresh status before retrying."
        case .result: "The Node returned an unexpected result."
        case .remote(_, let message): message
        }
    }
}
