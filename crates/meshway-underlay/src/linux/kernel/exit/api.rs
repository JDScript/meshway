/// The qualification was committed and read back. It authorizes release of
/// the already-admitted first packet, not a claim that conntrack exists yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectionAck {
    pub incarnation: super::ExitIncarnation,
}

/// Second-stage result. Failure is returned only after the qualification has
/// been closed; the caller must retire the corresponding data-plane flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtConfirmation {
    Confirmed {
        incarnation: super::ExitIncarnation,
        conntrack_id: u32,
    },
    ConfirmFailed {
        incarnation: super::ExitIncarnation,
    },
}
