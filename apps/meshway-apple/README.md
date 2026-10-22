# Meshway macOS App

macOS 26+ / arm64, Swift 6 strict concurrency. Two Xcode targets share the generated Rust XCFramework:

- `Meshway`: SwiftUI main window and MenuBarExtra panel; typed IPC client, System Extension activation and VPN preferences.
- `PacketTunnel`: Packet Tunnel **System Extension** hosting the unique Rust Node, Apple Host, utun packet path, DNS and bounded authenticated XPC sessions.

`meshway-apple-bridge/src/client/` is the App-side protocol adapter. `ClientRequest` encodes commands and decodes responses with the existing IPC 13 codec before projecting UniFFI records/enums. It does not start a Node or contact the Server. Swift `FrameClient` and `Shared/FrameProtocol.swift` only transport bytes. App, streaming ping and `--probe` share the same adapter; generated Swift bindings are never hand-edited.

The 316 pt menu panel and main-window toolbar share `StatusIsland`: connection state, copyable Mesh IPv4 and the existing connection switch. An amber, single-line error opens a popover with independent Node/network errors. The panel uses compact menu rows for exit profiles and devices, showing five devices before expansion. Device rows copy their address; their dot distinguishes direct, relay, idle, offline and unreachable states.

The sidebar-free main window has Devices / Exit / Diagnostics segments. Its compact device table keeps a stable 210 pt detail column, revealed on selection without moving table columns. Settings (`⌘,`, SwiftUI `Settings`) contains the icon picker, account/local addresses, key rotation and confirmed logout, lease/sync/data-plane/NE details, and netcheck. Diagnostics uses fixed-height result areas. Missing measurements stay missing; profile checkmarks require the installed netmap to be applied. Profile exit names describe configured references, not a measured traffic path. Lab operations remain under `--lab` and the Debug menu.

State, segment, detail, list and copy transitions respect Reduce Motion. Connecting shows phase-derived steps, a pulsing dot and skeleton rows; Reduce Motion makes them static. `running + preparing` means applying, not proof of channel establishment. Only `running + ready` is connected. The switch keeps its teal on-state even when a preview disables interaction. The state projection is documented in the [App interface specification](../../docs/v2/node/apple-design.md#app-界面).

Diagnostics presents the ping decision's actual path/profile/rule/action, up to 24 recent RTT slots, and median/range/received/timeout statistics. Setup-inclusive echoes are distinguished and excluded from RTT aggregates; failures without successful RTT remain empty slots. Missing data stays unmeasured. Netcheck conclusions come from the typed report, including destination-dependent NAT mapping and fresh public mappings. Both diagnostic failures and result areas have fixed bounds. Shared glass empty states cover no peers, search, Mesh-only profiles and an unavailable Server. Cached operation is claimed only with a future lease, matching applied netmap and a running/ready data plane. The default-profile recovery link appears below a non-default current profile.

## Application icons

**Settings → General → Application icon** offers Ink, Frost, Indigo and Graphite, including before enrollment. Frost (variant 2, light glass with teal) is the default Finder/Launchpad/bundle icon. `App/Assets.xcassets/AppIcon.appiconset` supplies all macOS slots (16/32/128/256/512 pt, 1×/2×); the four `AppIconChoice-*.imageset` resources retain the processed 1024×1024 masters.

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

For Swift-only work, `MESHWAY_REUSE_CORE=1` skips `build-core.sh` and requires an existing `build/MeshwayCore.xcframework` plus its matching `build/bindings/meshway_apple_bridge.swift`. Use this flag with both `check.sh` and `sign-lab.sh` when reusing a verified core; it does not validate source freshness or convert Release into Debug. Protocol/core changes require the normal build. Version: 0.1 (17).

## Debug-only model and visual checks

```sh
APP=apps/meshway-apple/build/DerivedData/Build/Products/Debug/Meshway.app/Contents/MacOS/Meshway
"$APP" --model-tests
"$APP" --preview running --snapshot-dir /absolute/path/to/evidence
```

Preview states: `unenrolled`, `approval`, `running`, `error`, `connecting-auth`, `connecting-sync`, `connecting-apply`, `connecting-plane`, `no-peers`, `mesh-only`, `server-cached`, `lease-expired`. Rust constructs actual `ipc::Status` values and serializes them with the production codec; Swift decodes them with the production client. Main, panel and settings windows have a small **SAMPLE** corner mark. Preview bypasses extension activation, VPN preferences, XPC polling and browser approval. These entry points and the mark are excluded from Release builds. PNGs capture the actual WindowServer-composited SwiftUI views for manual review.

Use `--preview-section peers|exit|diagnostics`, `--preview-selected` for the first device's detail, and `--preview-settings general|account|details|diagnostics` for settings. Add `--dark` for dark appearance. Add `--preview-locale en|zh-Hans` to choose the App language, and `--preview-popover` to also capture the status popover. `--issue-codes` prints the error and warning registry as JSON (see `docs/v2/node/apple-design.md#issue-codes`). Each snapshot run captures main, panel and settings windows. Preview/model checks require the Debug Rust core (`MESHWAY_CORE_PROFILE=debug`); an optimized core retains binding metadata but rejects sample requests and contains no fixture values. Run host previews from the unsigned build; if the host rejects the profile-less development signature, use an ad-hoc-signed copy of the app for preview, preserving the development-signed product for the VM.

`--preview-search QUERY` filters the codec sample to exercise search-empty UI. `--preview-diagnostics ping|ping-exit|ping-denied|ping-unsupported|netcheck` (comma-separated) decodes Rust-generated Ping event frames, in Node stream order, and a Netcheck report through one production request each. The `running` sample has six peers (direct, relay, idle, relay-idle, offline). Swift does not manufacture protocol fixtures. Model tests also use separate typed unit inputs to check presentation calculations; those inputs are not preview data.

Preview deliberately disables system-changing buttons, even for a running sample. In the live App, selection readiness comes from Status (`desiredUp`, identity, netmap and instance); Select also respects the assignment lock, and Use default requires a persistent requested profile. Rotate key requires an identity and excludes `leaving`; Logout requires an identity. In-flight operations disable duplicate submissions. These are display conditions: the Node still validates its current state and CAS for every command, and returns any additional refusal as a typed error.

The VM-only `--lab` management and `--probe status|up|down|exit PROFILE|exit --default|ping NAME` remain available for existing acceptance procedures. Ping cancels after five seconds and requires at least two events plus a cancellation result. `--lab inspect-preferences` reads the existing provider identifier and capture flags without saving preferences. `--probe` prints the Rust-validated response and is not the packaged production CLI.

Environment, signing and deployment: [`macos-lab.md`](../../docs/v2/node/macos-lab.md). Architecture and implementation boundaries: [`apple-design.md`](../../docs/v2/node/apple-design.md). VM GUI operation and packet-flow acceptance remain separate from codec/model tests and preview screenshots.
