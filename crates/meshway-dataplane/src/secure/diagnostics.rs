//! Diagnostic-only budget. It never participates in admission or authority.
use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

pub(crate) fn event() -> bool {
    static BUDGET: OnceLock<Mutex<(Instant, u32, u32)>> = OnceLock::new();
    let Ok(mut budget) = BUDGET
        .get_or_init(|| Mutex::new((Instant::now(), 0, 0)))
        .try_lock()
    else {
        return false;
    };
    if budget.0.elapsed() >= Duration::from_secs(1) {
        budget.0 = Instant::now();
        budget.1 = 0;
    }
    if budget.1 >= 64 || budget.2 >= 32_768 {
        return false;
    }
    budget.1 += 1;
    budget.2 += 1;
    true
}

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
