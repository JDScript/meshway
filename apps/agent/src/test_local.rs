//! Exact production write implementation with unit-test-only fsync fault hooks.

pub use meshway_protocol::local::{
    credential, digest, lock, open_private, private_dir, read_private, read_private_bounded,
    valid_credential,
};

#[path = "../../../crates/protocol/src/local/write.rs"]
mod write;
pub(crate) use write::test_faults;
pub use write::{AtomicWriteError, atomic_write, ensure_durable};
