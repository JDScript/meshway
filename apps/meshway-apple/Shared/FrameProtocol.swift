import Foundation

// Transport only. These bytes will be dispatched by meshway-node, not decoded in Swift.
@objc protocol FrameService {
    func send(_ frame: Data, reply: @escaping @Sendable (Bool) -> Void)
}

@objc protocol FrameReceiver {
    // Acknowledgement bounds server pushes even when the client stops consuming.
    func receive(_ frame: Data, reply: @escaping @Sendable () -> Void)
}

enum ShellConfiguration {
    static let maximumFrameSize = 65_536 + 4

    static func value(_ key: String) throws -> String {
        guard let value = Bundle.main.object(forInfoDictionaryKey: key) as? String,
              !value.isEmpty,
              value.unicodeScalars.allSatisfy({ CharacterSet.alphanumerics.contains($0) || ".-_".unicodeScalars.contains($0) })
        else { throw CocoaError(.coderInvalidValue) }
        return value
    }

    static func requirement(server: Bool) throws -> String {
        let team = try value("MeshwayTeam")
        let identifier = try value(server ? "MeshwayExtensionID" : "MeshwayAppID")
        // --probe runs the same signed App executable. A future embedded CLI must
        // add its exact identifier here; accepting every executable in the Team is too broad.
        return "anchor apple generic and certificate leaf[subject.OU] = \"\(team)\" and identifier \"\(identifier)\""
    }
}
