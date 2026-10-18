import Foundation
import Synchronization

// Lab client only: the server dispatches these IPC 13 requests in meshway-node.
// Each request uses a new authenticated XPC/Rust session, matching Unix IPC.
enum Probe {
    struct Command: Encodable {
        let type: String
        var expectedRevision: UInt64?
        var enrollmentKey: String?
        var profile: String?
        var target: Target?
        var count: UInt16?
    }
    struct Target: Encodable { let type = "name"; let name: String }
    struct Request: Encodable {
        let version = 13
        let requestId: String
        let command: Command
    }
    struct Response: Decodable {
        struct ResultBody: Decodable {
            struct Status: Decodable { let revision: UInt64; let server: String }
            let type: String
            let status: Status?
            let code: String?
        }
        let version: UInt16
        let requestId: String
        let result: ResultBody
    }

    static func request(_ command: Command) throws -> (String, Data) {
        let id = UUID().uuidString.lowercased()
        let body = try JSONEncoder().encode(Request(requestId: id, command: command))
        guard body.count <= 65_536 else { throw CocoaError(.coderInvalidValue) }
        var length = UInt32(body.count).bigEndian
        var frame = withUnsafeBytes(of: &length) { Data($0) }
        frame.append(body)
        return (id, frame)
    }
    static func body(_ frame: Data) throws -> Data {
        guard frame.count >= 4 else { throw CocoaError(.coderReadCorrupt) }
        let length = frame.prefix(4).reduce(UInt32(0)) { ($0 << 8) | UInt32($1) }
        guard length > 0, length <= 65_536, frame.count == Int(length) + 4 else { throw CocoaError(.coderReadCorrupt) }
        return Data(frame.dropFirst(4))
    }
    static func exchange(_ command: Command) async throws -> (Response, Data) {
        let (id, frame) = try request(command)
        let operation = ProbeExchange()
        let reply = try await operation.run(frame)
        let data = try body(reply)
        let response = try JSONDecoder().decode(Response.self, from: data)
        guard response.version == 13, response.requestId == id else { throw CocoaError(.coderReadCorrupt) }
        return (response, data)
    }

    @MainActor static func run() {
        Task {
            do {
                let arguments = Array(CommandLine.arguments.drop { $0 != "--probe" }.dropFirst())
                let action = arguments.first ?? "status"
                guard ["status", "up", "down", "exit", "ping"].contains(action) else { throw CocoaError(.coderInvalidValue) }
                if action == "ping" {
                    guard arguments.count == 2 else { throw CocoaError(.coderInvalidValue) }
                    let (id, frame) = try request(Command(type: "ping", target: Target(name: arguments[1]), count: 1000))
                    let received = Mutex(0)
                    let failed = Mutex(false)
                    let client = try FrameClient(output: { frame, acknowledge in
                        defer { acknowledge() }
                        do {
                            let data = try body(frame)
                            let response = try JSONDecoder().decode(Response.self, from: data)
                            guard response.version == 13, response.requestId == id, response.result.type == "ping" else { throw CocoaError(.coderReadCorrupt) }
                            received.withLock { $0 += 1 }
                            FileHandle.standardOutput.write(data + Data([10]))
                        } catch { failed.withLock { $0 = true } }
                    }, disconnected: { failed.withLock { $0 = true } })
                    client.send(frame) { accepted in if !accepted { failed.withLock { $0 = true } } }
                    try await Task.sleep(for: .seconds(5))
                    let success = received.withLock { $0 >= 2 } && !failed.withLock { $0 }
                    client.close()
                    print("probe stream disconnected after five seconds")
                    exit(success ? 0 : 1)
                }
                let (status, statusData) = try await exchange(Command(type: "status"))
                guard let current = status.result.status else { throw CocoaError(.coderReadCorrupt) }
                if action == "status" {
                    print(String(decoding: statusData, as: UTF8.self)); exit(0)
                }
                if action == "exit" {
                    guard arguments.count == 2 else { throw CocoaError(.coderInvalidValue) }
                    let command = arguments[1] == "--default" ? Command(type: "clearProfile") : Command(type: "selectProfile", profile: arguments[1])
                    let (response, data) = try await exchange(command)
                    print(String(decoding: data, as: UTF8.self))
                    exit(response.result.type == "error" ? 1 : 0)
                }
                var key: String?
                if let index = arguments.firstIndex(of: "--enrollment-key") {
                    guard action == "up", index + 1 < arguments.count else { throw CocoaError(.coderInvalidValue) }
                    let source = arguments[index + 1]
                    guard source.hasPrefix("@") else { throw CocoaError(.coderInvalidValue) }
                    let file = try FileHandle(forReadingFrom: URL(fileURLWithPath: String(source.dropFirst())))
                    defer { try? file.close() }
                    let data = try file.read(upToCount: 65_537) ?? Data()
                    guard data.count <= 65_536, let text = String(data: data, encoding: .utf8) else { throw CocoaError(.coderInvalidValue) }
                    key = text.trimmingCharacters(in: .whitespacesAndNewlines)
                }
                if let index = arguments.firstIndex(of: "--server") {
                    guard index + 1 < arguments.count else { throw CocoaError(.coderInvalidValue) }
                    let origin = arguments[index + 1].hasSuffix("/") ? arguments[index + 1] : arguments[index + 1] + "/"
                    guard origin == current.server else {
                        print("Server differs from running Node; start the tunnel with MESHWAY_SERVER_URL first"); exit(2)
                    }
                }
                let (response, data) = try await exchange(Command(type: action, expectedRevision: current.revision, enrollmentKey: key))
                print(String(decoding: data, as: UTF8.self))
                exit(response.result.type == "error" ? 1 : 0)
            } catch { print("probe error: \(error)"); exit(1) }
        }
        dispatchMain()
    }
}

private final class ProbeExchange: @unchecked Sendable {
    private struct State {
        var continuation: CheckedContinuation<Data, Error>?
        var client: FrameClient?
    }
    private let state = Mutex(State())
    func run(_ frame: Data) async throws -> Data {
        try await withCheckedThrowingContinuation { continuation in
            state.withLock { $0.continuation = continuation }
            do {
                let client = try FrameClient(output: { [weak self] frame, acknowledge in
                    acknowledge()
                    self?.finish(.success(frame))
                }, disconnected: { [weak self] in self?.finish(.failure(CocoaError(.fileReadUnknown))) })
                let active = state.withLock { state in
                    guard state.continuation != nil else { return false }
                    state.client = client
                    return true
                }
                guard active else { client.close(); return }
                client.send(frame) { [weak self] accepted in
                    if !accepted { self?.finish(.failure(CocoaError(.fileWriteUnknown))) }
                }
                DispatchQueue.global().asyncAfter(deadline: .now() + 5) { [weak self] in
                    self?.finish(.failure(CocoaError(.fileReadUnknown)))
                }
            } catch { finish(.failure(error)) }
        }
    }
    private func finish(_ result: Result<Data, Error>) {
        let (continuation, client) = state.withLock { state in
            let owned = (state.continuation, state.client)
            state.continuation = nil; state.client = nil
            return owned
        }
        continuation?.resume(with: result)
        client?.close()
    }
}
