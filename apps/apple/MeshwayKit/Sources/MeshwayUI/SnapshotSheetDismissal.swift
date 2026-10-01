#if DEBUG && os(macOS)
  import Foundation
  import SwiftUI

  struct SnapshotSheetDismissal: ViewModifier {
    let dismiss: DismissAction
    func body(content: Content) -> some View {
      content.onReceive(
        NotificationCenter.default.publisher(for: Notification.Name("MeshwaySnapshotAction"))
      ) {
        guard ProcessInfo.processInfo.environment["MESHWAY_SNAPSHOT_DIR"] != nil,
          $0.object as? String == "dismiss"
        else { return }
        // Use the same presentation action as the sheet's actual Done button.
        // A covered root view is not a reliable recipient of capture IPC.
        dismiss()
      }
    }
  }
#endif
