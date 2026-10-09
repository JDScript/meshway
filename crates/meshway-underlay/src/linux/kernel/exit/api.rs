/// The qualification was committed and read back. It authorizes release of
/// the already-admitted first packet, not a claim that conntrack exists yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectionAck {
    pub incarnation: u128,
}

/// Second-stage result. Failure is returned only after the qualification has
/// been closed; the caller must retire the corresponding data-plane flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtConfirmation {
    Confirmed {
        incarnation: u128,
        conntrack_id: u32,
    },
    ConfirmFailed {
        incarnation: u128,
    },
}
