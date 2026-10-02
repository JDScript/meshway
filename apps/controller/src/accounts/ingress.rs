//! Shared bounded KDF worker and account/source admission budget.
use super::{AccountError, PasswordWorker, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

pub(crate) struct Ingress {
    worker: PasswordWorker,
    budget: Mutex<RateBudget>,
}
struct RateBudget {
    start: Instant,
    total: u32,
    keys: HashMap<[u8; 32], u32>,
}
impl Ingress {
    pub(crate) fn new() -> Result<Self> {
        Ok(Self {
            worker: PasswordWorker::new_blocking(2)?,
            budget: Mutex::new(RateBudget {
                start: Instant::now(),
                total: 0,
                keys: HashMap::new(),
            }),
        })
    }
    pub(crate) fn worker(&self) -> &PasswordWorker {
        &self.worker
    }
    pub(crate) fn reserve(&self, source: Option<IpAddr>, account: &str) -> Result<()> {
        let mut budget = self.budget.lock().map_err(|_| AccountError::Worker)?;
        if budget.start.elapsed() >= Duration::from_secs(60) {
            budget.start = Instant::now();
            budget.total = 0;
            budget.keys.clear();
        }
        let source = source.map_or_else(|| "unknown-source".into(), |s| s.to_string());
        let source: [u8; 32] = Sha256::digest(format!("source:{source}")).into();
        let account: [u8; 32] = Sha256::digest(format!("account:{account}")).into();
        if budget.total >= 120
            || budget.keys.len() >= 4096
            || budget.keys.get(&source).copied().unwrap_or(0) >= 30
            || budget.keys.get(&account).copied().unwrap_or(0) >= 10
        {
            return Err(AccountError::Busy);
        }
        budget.total += 1;
        *budget.keys.entry(source).or_default() += 1;
        *budget.keys.entry(account).or_default() += 1;
        Ok(())
    }
}
