import Foundation

// Lab stdout retains the complete Rust-validated response. No Swift JSON codec.
enum Probe {
    @MainActor static func run() {
        Task {
            do {
                let arguments = Array(CommandLine.arguments.drop { $0 != "--probe" }.dropFirst())
                let action = arguments.first ?? "status"
                guard ["status", "up", "down"].contains(action) else { throw IPCFailure.result }
                let operation = IPCExchange()
                guard case .status(let status) = try await operation.run(.status, text: true) else { throw IPCFailure.result }
                if action == "status" { print(operation.responseText ?? ""); exit(0) }
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
