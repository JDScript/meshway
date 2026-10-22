import SwiftUI

enum AppIconChoice: String, CaseIterable, Identifiable {
    case ink = "1-ink", light = "2-light", gradient = "3-gradient", mono = "4-mono"
    var id: String { rawValue }
    var assetName: String { "AppIconChoice-\(rawValue)" }
    var title: String {
        switch self {
        case .ink: "Ink"
        case .light: "Frost"
        case .gradient: "Indigo"
        case .mono: "Graphite"
        }
    }
}

@MainActor @Observable
final class IconPreferences {
    static let shared = IconPreferences()
    private static let key = "applicationIconChoice"
    private let defaults: UserDefaults?
    private(set) var choice: AppIconChoice

    // Preview gets an in-memory preference and never changes the Dock or defaults.
    init(defaults: UserDefaults? = .standard) {
        self.defaults = defaults
        choice = defaults?.string(forKey: Self.key).flatMap(AppIconChoice.init(rawValue:)) ?? .light
    }

    func select(_ choice: AppIconChoice) {
        self.choice = choice
        defaults?.set(choice.rawValue, forKey: Self.key)
        apply()
    }

    func apply() {
        guard defaults != nil, let image = NSImage(named: choice.assetName) else { return }
        NSApplication.shared.applicationIconImage = image
    }
}

struct IconPicker: View {
    @Environment(IconPreferences.self) private var preferences
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 12) {
                ForEach(AppIconChoice.allCases) { choice in
                    let selected = preferences.choice == choice
                    Button { preferences.select(choice) } label: {
                        VStack(spacing: 4) {
                            Image(choice.assetName).resizable().interpolation(.high)
                                .scaledToFit().frame(width: 76, height: 76)
                                .scaleEffect(selected ? 1 : 0.94)
                            Text(verbatim: choice.title).font(.callout.weight(selected ? .semibold : .regular))
                            Image(systemName: "checkmark.circle.fill")
                                .font(.caption).foregroundStyle(.tint)
                                .opacity(selected ? 1 : 0).accessibilityHidden(true)
                        }
                        .frame(maxWidth: .infinity).padding(.vertical, 10)
                        .background(selected ? Color.accentColor.opacity(0.09) : .clear, in: .rect(cornerRadius: 12))
                        .overlay { RoundedRectangle(cornerRadius: 12).strokeBorder(selected ? Color.accentColor.opacity(0.55) : .clear, lineWidth: 1) }
                        .contentShape(.rect(cornerRadius: 12))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(choice == .light ? Text("\(choice.title), default") : Text(verbatim: choice.title))
                    .accessibilityAddTraits(selected ? .isSelected : [])
                    .help(choice == .light ? Text("Frost · default") : Text("Use \(choice.title) in the Dock"))
                }
            }
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: preferences.choice)
            Text("Shown in the Dock while Meshway runs.")
                .font(.caption).foregroundStyle(.secondary).lineLimit(nil).fixedSize(horizontal: false, vertical: true)
        }.padding(.vertical, 4)
    }
}
