//! Build identity of the executable this crate is linked into: the one
//! `meshway version` shows and an input of the Server's netmap compilation
//! (`docs/v2/node/netmap-design.md`).

/// Workspace version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// `git describe --tags --always --dirty --abbrev=8` of the build: from
/// `MESHWAY_GIT_DESCRIBE` when the release workflow sets it, else from Git;
/// `unknown` without either.
pub const GIT: &str = env!("MESHWAY_BUILD_GIT");
/// Target triple of the build.
pub const TARGET: &str = env!("MESHWAY_BUILD_TARGET");

/// Version and Git description together; two builds with the same identity
/// count as the same code.
pub fn identity() -> String {
    format!("{VERSION} ({GIT})")
}
