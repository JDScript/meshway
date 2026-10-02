use std::sync::Arc;

use argon2::{
    Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version,
    password_hash::SaltString,
};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

use super::{AccountError, HashedPassword, Password, Result, UserId};

pub(crate) struct CredentialSnapshot {
    pub(super) user: UserId,
    pub(super) version: i64,
    pub(super) phc: Zeroizing<String>,
}

/// Only the bounded KDF worker can produce a successful password proof.
pub(crate) struct VerifiedCredential {
    pub(super) user: UserId,
    pub(super) version: i64,
}

/// Non-queueing budget: at most two 64 MiB KDFs by default (128 MiB),
/// Argon2id v19 m=65536 KiB/t=3/p=1. Never call under a Store/SQLite lock.
/// The permit moves into spawn_blocking, so cancelling HTTP does not release it
/// while CPU work is still running. Ingress rate limiting is additionally required.
#[derive(Clone)]
pub(crate) struct PasswordWorker {
    pub(super) budget: Arc<Semaphore>,
    dummy: Arc<Zeroizing<String>>,
}

fn argon() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(65536, 3, 1, Some(32)).expect("fixed Argon2 parameters"),
    )
}

fn hash(password: &Password) -> Result<HashedPassword> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|_| AccountError::Worker)?;
    let salt = SaltString::encode_b64(&salt).map_err(|_| AccountError::Worker)?;
    let phc = argon()
        .hash_password(password.0.as_bytes(), &salt)
        .map_err(|_| AccountError::Worker)?;
    Ok(HashedPassword(Zeroizing::new(phc.to_string())))
}

impl PasswordWorker {
    /// Initialize once at controller startup, outside its database lock.
    #[cfg(test)]
    pub(crate) async fn new(max_parallel: usize) -> Result<Self> {
        tokio::task::spawn_blocking(move || Self::new_blocking(max_parallel))
            .await
            .map_err(|_| AccountError::Worker)?
    }

    pub(crate) fn new_blocking(max_parallel: usize) -> Result<Self> {
        if !(1..=8).contains(&max_parallel) {
            return Err(AccountError::Invalid);
        }
        let dummy = hash(&Password::new(
            meshway_protocol::local::credential().map_err(|_| AccountError::Worker)?,
        )?)?;
        Ok(Self {
            budget: Arc::new(Semaphore::new(max_parallel)),
            dummy: Arc::new(dummy.0),
        })
    }

    pub(crate) async fn hash(&self, password: Password) -> Result<HashedPassword> {
        let permit = self
            .budget
            .clone()
            .try_acquire_owned()
            .map_err(|_| AccountError::Busy)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            hash(&password)
        })
        .await
        .map_err(|_| AccountError::Worker)?
    }

    pub(crate) async fn verify(
        &self,
        snapshot: Option<CredentialSnapshot>,
        password: Password,
    ) -> Result<VerifiedCredential> {
        let permit = self
            .budget
            .clone()
            .try_acquire_owned()
            .map_err(|_| AccountError::Busy)?;
        let dummy = self.dummy.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let phc = snapshot.as_ref().map_or(dummy.as_str(), |s| s.phc.as_str());
            let parsed = PasswordHash::new(phc).map_err(|_| AccountError::Worker)?;
            // Accept only the locally generated, bounded parameter set. Corrupt
            // persisted parameters must not turn login into unbounded work.
            if parsed.algorithm.as_str() != "argon2id"
                || parsed.version != Some(19)
                || parsed.params.get_decimal("m") != Some(65536)
                || parsed.params.get_decimal("t") != Some(3)
                || parsed.params.get_decimal("p") != Some(1)
            {
                return Err(AccountError::Worker);
            }
            argon()
                .verify_password(password.0.as_bytes(), &parsed)
                .map_err(|_| AccountError::Unauthorized)?;
            let snapshot = snapshot.ok_or(AccountError::Unauthorized)?;
            Ok(VerifiedCredential {
                user: snapshot.user,
                version: snapshot.version,
            })
        })
        .await
        .map_err(|_| AccountError::Worker)?
    }
}
