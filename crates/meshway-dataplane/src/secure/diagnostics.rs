//! Secure adapter's bounded engine rejection labels.
pub(super) use crate::diagnostics::event;

pub(super) fn engine_error(error: &crate::Error) -> &'static str {
    match error {
        crate::Error::Packet(_) => "packet",
        crate::Error::FactsMismatch => "facts_mismatch",
        crate::Error::Control(_) => "control",
        crate::Error::Admission(_) => "admission",
        crate::Error::Limits => "limits",
        crate::Error::Ingress => "ingress",
        crate::Error::Sender => "sender",
        crate::Error::Flow => "flow",
        crate::Error::Capacity => "capacity",
        crate::Error::Expired => "expired",
        crate::Error::ExecutionScope => "execution_scope",
    }
}
