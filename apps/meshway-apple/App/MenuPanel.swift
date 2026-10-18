import SwiftUI

struct MenuPanel: View {
    let model: AppModel
    @Environment(\.openWindow) private var openWindow
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            ConnectionHeader(model: model, compact: true)
            if model.sampleMode { SampleBanner() }
            Divider()
            if let status = model.status {
                if let node = status.node {
                    Text(node.hostname).font(.headline)
                    if let address = node.ipv4 ?? node.ipv6 {
                        HStack { Text(address).font(.callout.monospaced()).foregroundStyle(.secondary); Spacer(); Button { copy(address) } label: { Image(systemName: "doc.on.doc") }.buttonStyle(.plain) }
                    }
                }
                if status.phase == .awaitingApproval {
                    Text("Approve this Mac in your browser. It will connect automatically.").font(.callout).foregroundStyle(.secondary)
                }
                if let error = status.lastError { Label(human(error), systemImage: "exclamationmark.circle").foregroundStyle(.orange) }
                if let error = status.networkError { Text("Network · \(human(error.problem))").foregroundStyle(.orange) }
                if let selection = model.selection { LabeledContent("Exit profile", value: selection.appliedProfile + (model.netmapApplied ? "" : " · not applied")).font(.callout) }
                if !status.peers.isEmpty {
                    Text("PEERS").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    ForEach(Array(status.peers.prefix(3)), id: \.nodeId) { peer in PeerRow(model: model, peer: peer).contextMenu { PeerCopyActions(peer: peer) } }
                }
            } else {
                Label("Set up this Mac", systemImage: "laptopcomputer").font(.headline)
                Text("Open Meshway to enter your Server URL and enable the network extension.").font(.callout).foregroundStyle(.secondary)
            }
            if let issue = model.issue ?? model.ipcIssue { Text(issue).font(.caption).foregroundStyle(.orange).lineLimit(3) }
            Divider()
            HStack {
                Button("Open Meshway", systemImage: "arrow.up.forward.app") { openWindow(id: "main"); NSApp.activate() }
                Spacer()
                Menu { Button("Quit Meshway") { NSApp.terminate(nil) } } label: { Image(systemName: "ellipsis.circle") }.menuStyle(.borderlessButton).fixedSize()
            }
        }.padding(20).frame(width: 380)
            .background(Color(nsColor: .windowBackgroundColor))
    }
}
