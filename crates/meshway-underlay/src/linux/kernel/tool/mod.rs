//! External kernel tools shared by preflight, the exit role and source capture:
//! bounded joinable processes, their crash fence and strict nft readback.
pub(super) mod child;
pub(super) mod command;
pub(super) mod nft;
