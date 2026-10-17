# Meshway macOS shell

macOS 26+ / arm64, Swift 6 strict concurrency. Two Xcode targets:

- `Meshway`: SwiftUI menu bar and debug window, System Extension activation, VPN preferences, XPC client. `Meshway.app/Contents/MacOS/Meshway --probe` is the CLI transport probe.
- `PacketTunnel`: Packet Tunnel **System Extension**, test route settings and bounded XPC echo/push fixture. It does not link a Node runtime or forward packets.

The synchronized `Shared/` folder contains only the opaque frame transport and code-signing requirement. `FrameSession.dispatchFrame` is the future Rust dispatch entry; `cancel` owns connection-scoped stream cancellation. Fixture bytes are not IPC 13 Status.

```sh
bash apps/meshway-apple/scripts/check.sh
bash apps/meshway-apple/scripts/build.sh
```

`check.sh` compiles both targets with `CODE_SIGNING_ALLOWED=NO`. Configure signing in ignored `Config/Local.xcconfig`; no Team is committed. `sign-lab.sh` explicitly creates the profile-less development-signed VM experiment. `deploy-vm.sh` installs only inside the named Tart VM.

Full environment, Recovery, signing, deployment and cleanup reference: [`docs/v2/node/macos-lab.md`](../../docs/v2/node/macos-lab.md). Component boundaries and acceptance: [`apple-design.md`](../../docs/v2/node/apple-design.md).
