# Meshway macOS App

macOS 26+ / arm64, Swift 6 strict concurrency. Two Xcode targets share the generated Rust XCFramework:

- `Meshway`: SwiftUI main window and MenuBarExtra panel; typed IPC client, System Extension activation and VPN preferences.
- `PacketTunnel`: Packet Tunnel **System Extension** hosting the unique Rust Node, Apple Host, utun packet path, DNS and bounded authenticated XPC sessions.

`meshway-apple-bridge/src/client/` is the App-side protocol adapter. `ClientRequest` encodes commands and decodes responses with the existing IPC 13 codec before projecting UniFFI records/enums. It does not start a Node or contact the Server. Swift `FrameClient` and `Shared/FrameProtocol.swift` only transport bytes. App, streaming ping and `--probe` share the same adapter; generated Swift bindings are never hand-edited.

The UI shows Node phase separately from macOS VPN state, enrollment approval, installed local addresses, lease/sync/errors, searchable peers and path details, routing profiles, netcheck, ping, key rotation and confirmed logout. Missing measurements stay missing. A committed routing selection is not shown as applied until the Node's netmap says so. Lab operations remain under `--lab` and the Debug menu.

## Application icons

**This Mac → Application icon** offers Ink, Frost, Indigo and Graphite, including before enrollment. Frost (variant 2, light glass with teal) is the default Finder/Launchpad/bundle icon. `App/Assets.xcassets/AppIcon.appiconset` supplies all macOS slots (16/32/128/256/512 pt, 1×/2×); the four `AppIconChoice-*.imageset` resources retain the processed 1024×1024 masters.

`IconPreferences` persists the choice in `UserDefaults` and applies `NSApplication.shared.applicationIconImage` at launch and on selection. The custom icon appears only in the running App's Dock tile; Finder keeps Frost. No App bundle files or filesystem icons are changed. The selector reserves its selection indicator space and respects Reduce Motion. Preview uses an isolated in-memory preference.

Regenerate from the four approved `variant-{1-ink,2-light,3-gradient,4-mono}.png` source files using Python with Pillow 12.1.0:

```sh
python apps/meshway-apple/scripts/generate-icons.py --source-dir /path/to/originals --evidence-dir /path/to/evidence
```

The script crops each glass tile, applies an antialiased inner-edge rounded mask to remove exterior alpha debris, preserves its aspect ratio, and centers it at a maximum 824×824 on a transparent 1024×1024 canvas. It adds no exterior shadow. Asset PNGs and catalog metadata are generated together; the optional evidence includes a contact sheet and 16/32/64/128 px exports.

## Build and check

Check disk space (>50 GiB) before building. The scripts use mise's locked Rust and a worktree-local Cargo target with incremental compilation disabled.

```sh
MESHWAY_CORE_PROFILE=debug bash apps/meshway-apple/scripts/check.sh
MESHWAY_CORE_PROFILE=debug bash apps/meshway-apple/scripts/sign-lab.sh
mise exec -- cargo test -p meshway-apple-bridge --lib
mise run check
```

`check.sh` compiles both targets unsigned. Configure development signing in ignored `Config/Local.xcconfig`. `sign-lab.sh` explicitly creates the profile-less development-signed VM experiment; this is not Developer ID distribution. `deploy-vm.sh` installs only inside the named Tart VM. Preserve an existing approved VPN manager when updating or restarting.

## Debug-only model and visual checks

```sh
APP=apps/meshway-apple/build/DerivedData/Build/Products/Debug/Meshway.app/Contents/MacOS/Meshway
"$APP" --model-tests
"$APP" --preview running --snapshot-dir /absolute/path/to/evidence
```

Preview states: `unenrolled`, `approval`, `running`, `error`. Rust constructs actual `ipc::Status` values and serializes them with the production codec; Swift decodes them with the production client. The main window and compact panel display **SAMPLE DATA**. Preview bypasses extension activation, VPN preferences, XPC polling and browser approval. These entry points are excluded from Release builds. PNGs are rendered from the actual AppKit-hosted SwiftUI views for manual review.

Use `--preview-section peers`, `--preview-section diagnostics` or `--preview-section device` (icon selector) for a main-window capture; add `--dark` for dark appearance. The fixed sample footer remains visible. Preview/model checks require the Debug Rust core (`MESHWAY_CORE_PROFILE=debug`); an optimized core retains binding metadata but rejects sample requests and contains no fixture values.

Preview deliberately disables system-changing buttons, even for a running sample. In the live App, selection readiness comes from Status (`desiredUp`, identity, netmap and instance); Select also respects the assignment lock, and Use default requires a persistent requested profile. Rotate key requires an identity and excludes `leaving`; Logout requires an identity. In-flight operations disable duplicate submissions. These are display conditions: the Node still validates its current state and CAS for every command, and returns any additional refusal as a typed error.

The VM-only `--lab` management and `--probe status|up|down|exit PROFILE|exit --default|ping NAME` remain available for existing acceptance procedures. Ping cancels after five seconds and requires at least two events plus a cancellation result. `--lab inspect-preferences` reads the existing provider identifier and capture flags without saving preferences. `--probe` prints the Rust-validated response and is not the packaged production CLI.

Environment, signing and deployment: [`macos-lab.md`](../../docs/v2/node/macos-lab.md). Architecture and implementation boundaries: [`apple-design.md`](../../docs/v2/node/apple-design.md). VM GUI operation and packet-flow acceptance remain separate from codec/model tests and preview screenshots.
