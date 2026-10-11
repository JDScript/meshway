use argon2::{
    Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version,
    password_hash::SaltString,
};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

use super::{PasswordRules, Secret};

use crate::error::Error;
use crate::store::accounts::LoginCandidate;

pub(super) const RULES: PasswordRules = PasswordRules {
    min_codepoints: 12,
    max_codepoints: 128,
    max_utf8_bytes: 512,
};
// Process-wide, including separate Apps. A burst waits briefly for a slot
// (at most MAX_WAITERS callers, each at most KDF_WAIT) instead of failing at
// once; beyond that it is Busy, so neither passwords nor blocking tasks queue
// without bound.
const MAX_KDFS: usize = 4;
static KDFS: Semaphore = Semaphore::const_new(MAX_KDFS);
const MAX_WAITERS: usize = 64;
static WAITERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
const KDF_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
#[cfg(test)]
pub(crate) static TEST_KDF_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MEMORY_KIB: u32 = 19456;
const ITERATIONS: u32 = 2;
const LANES: u32 = 1;
// Not an account credential. Equal-cost verification for unknown identifiers;
// its result is never used to grant a session.
const DUMMY: &str = "$argon2id$v=19$m=19456,t=2,p=1$AAAAAAAAAAAAAAAAAAAAAA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

pub(super) fn validate(password: &Secret) -> Result<(), Error> {
    let value = password.expose();
    if value.len() > RULES.max_utf8_bytes
        || !(RULES.min_codepoints..=RULES.max_codepoints).contains(&value.chars().count())
    {
        return Err(Error::InvalidPassword(RULES));
    }
    Ok(())
}

fn algorithm() -> Result<Argon2<'static>, Error> {
    let params = Params::new(MEMORY_KIB, ITERATIONS, LANES, Some(32))
        .map_err(|_| Error::Internal("argon2 parameters"))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// This first storage format accepts exactly the production KDF profile. No PHC
/// supplied memory/iteration values can increase the per-task work bound.
fn bounded_phc(value: &str) -> Result<PasswordHash<'_>, Error> {
    if value.len() > 256 {
        return Err(Error::InvalidCredentials);
    }
    let hash = PasswordHash::new(value).map_err(|_| Error::InvalidCredentials)?;
    if hash.algorithm.as_str() != "argon2id"
        || hash.version != Some(19)
        || hash.params.iter().count() != 3
        || hash.params.get_decimal("m") != Some(MEMORY_KIB)
        || hash.params.get_decimal("t") != Some(ITERATIONS)
        || hash.params.get_decimal("p") != Some(LANES)
        || hash.hash.as_ref().map(|value| value.len()) != Some(32)
    {
        return Err(Error::InvalidCredentials);
    }
    let mut salt = [0u8; 64];
    let salt = hash
        .salt
        .ok_or(Error::InvalidCredentials)?
        .decode_b64(&mut salt)
        .map_err(|_| Error::InvalidCredentials)?;
    if salt.len() != 16 {
        return Err(Error::InvalidCredentials);
    }
    Ok(hash)
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    let permit = match KDFS.try_acquire() {
        Ok(permit) => permit,
        Err(_) => {
            use std::sync::atomic::Ordering;
            // Released on every exit, including a cancelled caller.
            struct Waiting;
            impl Drop for Waiting {
                fn drop(&mut self) {
                    WAITERS.fetch_sub(1, Ordering::AcqRel);
                }
            }
            let waiting = Waiting;
            if WAITERS.fetch_add(1, Ordering::AcqRel) >= MAX_WAITERS {
                return Err(Error::Busy);
            }
            let waited = tokio::time::timeout(KDF_WAIT, KDFS.acquire()).await;
            drop(waiting);
            waited
                .map_err(|_| Error::Busy)?
                .map_err(|_| Error::RUNTIME)?
        }
    };
    tokio::task::spawn_blocking(move || {
        // Cancellation of the caller never releases the actual KDF budget.
        let _permit = permit;
        work()
    })
    .await
    .map_err(|_| Error::RUNTIME)?
}

pub(super) async fn hash(password: Secret) -> Result<Zeroizing<String>, Error> {
    blocking(move || {
        let mut salt = Zeroizing::new([0u8; 16]);
        getrandom::fill(salt.as_mut()).map_err(|_| Error::ENTROPY)?;
        let salt = SaltString::encode_b64(salt.as_ref())
            .map_err(|_| Error::Internal("argon2 salt encoding"))?;
        Ok(Zeroizing::new(
            algorithm()?
                .hash_password(password.expose().as_bytes(), &salt)
                .map_err(|_| Error::Internal("argon2 password hashing"))?
                .to_string(),
        ))
    })
    .await
}

pub(super) async fn verify(
    password: Secret,
    candidate: Option<LoginCandidate>,
) -> Result<LoginCandidate, Error> {
    blocking(move || {
        let parsed = candidate
            .as_ref()
            .and_then(|c| bounded_phc(&c.password_hash).ok());
        let valid_hash = parsed.is_some();
        let parsed = parsed.unwrap_or(bounded_phc(DUMMY)?);
        let valid_password = algorithm()?
            .verify_password(password.expose().as_bytes(), &parsed)
            .is_ok();
        if valid_hash && valid_password {
            candidate.ok_or(Error::InvalidCredentials)
        } else {
            Err(Error::InvalidCredentials)
        }
    })
    .await
}

#[cfg(test)]
mod tests;
