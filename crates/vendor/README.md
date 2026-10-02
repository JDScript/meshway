# Fixed upstream dependency patch

`socket2/` is the exact crates.io `socket2 0.6.5` archive, with one Linux-only
safe method added: `Socket::netns_cookie()`. It is an external dependency, not
an additional Meshway workspace crate. Existing methods, defaults and features
are unchanged. Meshway workspace crates retain `unsafe_code = "forbid"`.

Upstream archive SHA-256 (the original Cargo.lock checksum):

```text
c3d1e2c7f27f8d4cb10542a02c49005dbd6e93095799d6f3be745fae9f8fedd4
```

`socket2-upstream-sha256.json` records every original archive file. Exactly
`src/sys/unix.rs` differs. The root Cargo patch pins resolution to this local
copy; upstream licenses and `.cargo_vcs_info.json` are retained. Keeping it
under `crates/vendor` lets existing Rust Docker source-copy boundaries include
the dependency without adding frontend/build-time network dependencies.

## Why this API

Underlay ownership must distinguish successive network namespace lifetimes.
The existing `(boot, nsfs device, inode)` tuple can repeat within one boot.
`SO_COOKIE` identifies a single socket and cannot be used instead.

`SO_NETNS_COOKIE` is a Linux 5.7+ socket option returning the kernel's 64-bit
network namespace lifetime identifier. This extension uses socket2's existing
typed getsockopt boundary with the same ABI type as its SO_COOKIE method.
It does not silently substitute a different identifier on unsupported kernels.
The caller must define boot binding, unsupported-kernel handling and legacy
journal migration; adding this method alone does not close the ownership gate.

The source addition and its Linux behavior require independent review before
integration. Do not remove the patch until an upstream version exposes an
equivalent safe API and the owner/recovery tests have been rerun.
