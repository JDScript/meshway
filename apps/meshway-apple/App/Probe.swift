import Foundation
import Synchronization

// Lab stdout retains the complete Rust-validated response. No Swift JSON codec.
enum Probe {
    @MainActor static func run() {
        Task {
            do {
                let arguments = Array(CommandLine.arguments.drop { $0 != "--probe" }.dropFirst())
                let action = arguments.first ?? "status"
                guard ["status", "up", "down", "exit", "ping"].contains(action) else { throw IPCFailure.result }
                if action == "ping" {
                    // Cancels the stream after five seconds; Rust must observe the disconnect.
                    guard arguments.count == 2 else { throw IPCFailure.result }
                    let received = Mutex(0)
                    let stream = Task { @MainActor in
                        try await IPCExchange().run(.ping(target: arguments[1], count: 1000), event: { event in
                            received.withLock { $0 += 1 }
                            print("ping event: \(event)")
                        })
                    }
                    try await Task.sleep(for: .seconds(5))
                    stream.cancel()
                    let outcome = await stream.result
                    let cancelled = if case .failure(let error) = outcome { error is CancellationError } else { false }
                    print("probe stream disconnected after five seconds")
                    exit(received.withLock { $0 >= 2 } && cancelled ? 0 : 1)
                }
                let operation = IPCExchange()
                guard case .status(let status) = try await operation.run(.status, text: true) else { throw IPCFailure.result }
                if action == "status" { print(operation.responseText ?? ""); exit(0) }
                if action == "exit" {
                    guard arguments.count == 2 else { throw IPCFailure.result }
                    let command: ClientCommand = arguments[1] == "--default" ? .clearProfile : .selectProfile(profile: arguments[1])
                    let result = try await operation.run(command, text: true)
                    print(operation.responseText ?? "")
                    if case .error = result { exit(1) }
                    exit(0)
                }
                var key: String?
                if let index = arguments.firstIndex(of: "--enrollment-key") {
                    guard action == "up", index + 1 < arguments.count, arguments[index + 1].hasPrefix("@") else { throw IPCFailure.result }
                    let file = try FileHandle(forReadingFrom: URL(fileURLWithPath: String(arguments[index + 1].dropFirst())))
                    defer { try? file.close() }
                    let data = try file.read(upToCount: 65_537) ?? Data()
                    guard data.count <= 65_536, let text = String(data: data, encoding: .utf8) else { throw IPCFailure.result }
                    key = text.trimmingCharacters(in: .whitespacesAndNewlines)
                }
                if let index = arguments.firstIndex(of: "--server") {
                    guard index + 1 < arguments.count else { throw IPCFailure.result }
                    let origin = arguments[index + 1].hasSuffix("/") ? arguments[index + 1] : arguments[index + 1] + "/"
                    guard origin == status.server else { print("Server differs from running Node; configure the tunnel first"); exit(2) }
                }
                let command: ClientCommand = action == "up" ? .up(revision: status.revision, enrollmentKey: key, reenroll: false) : .down(revision: status.revision)
                let result = try await operation.run(command, text: true)
                print(operation.responseText ?? "")
                if case .error = result { exit(1) }
                exit(0)
            } catch { print("probe error: \(error)"); exit(1) }
        }
        dispatchMain()
    }
}
