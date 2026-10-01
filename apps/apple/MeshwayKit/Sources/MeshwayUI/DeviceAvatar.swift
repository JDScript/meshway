import MeshwayCore
import SwiftUI

// Identity decoration only: no device-type or connectivity claims.
struct DeviceAvatar: View {
  let device: MeshDevice
  let isSelf: Bool
  private var initials: String {
    let words = device.name.split(whereSeparator: { !$0.isLetter && !$0.isNumber })
    guard let first = words.first?.first else { return "?" }
    let leading = String(String(first).uppercased().prefix(1))
    guard words.count > 1, let last = words.last?.first else { return leading }
    return leading + String(String(last).uppercased().prefix(1))
  }
  private var tint: Color {
    let palette: [Color] = [.teal, .blue, .indigo, .purple, .brown, .orange]
    // Stable across launches; Swift's randomized Hasher is unsuitable here.
    let hash = device.id.utf8.reduce(UInt64(14_695_981_039_346_656_037)) {
      ($0 ^ UInt64($1)) &* 1_099_511_628_211
    }
    return palette[Int(hash % UInt64(palette.count))]
  }
  var body: some View {
    Text(initials).font(.system(size: 10, weight: .semibold, design: .rounded))
      .foregroundStyle(tint)
      .frame(width: LayoutMetrics.iconSize, height: LayoutMetrics.iconSize)
      .background(tint.opacity(0.1), in: Circle())
      .overlay { Circle().strokeBorder(isSelf ? Color.indigo.opacity(0.4) : .clear, lineWidth: 1) }
      .accessibilityHidden(true)
  }
}
