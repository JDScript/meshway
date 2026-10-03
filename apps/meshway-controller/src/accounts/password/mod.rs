use argon2::{
    Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version,
    password_hash::SaltString,
};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

use super::{AccountError, PasswordRules, Secret};
use crate::store::accounts::LoginCandidate;

pub(super) const RULES: PasswordRules = PasswordRules {
    min_codepoints: 12,
    max_codepoints: 128,
    max_utf8_bytes: 512,
};
// Process-wide, including separate Apps. Admission is immediate (Busy), avoiding
// an unbounded queue of passwords or spawn_blocking tasks.
const MAX_KDFS: usize = 4;
static KDFS: Semaphore = Semaphore::const_new(MAX_KDFS);
#[cfg(test)]
pub(crate) static TEST_KDF_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MEMORY_KIB: u32 = 19456;
const ITERATIONS: u32 = 2;
const LANES: u32 = 1;
// Not an account credential. Equal-cost verification for unknown identifiers;
// its result is never used to grant a session.
const DUMMY: &str = "$argon2id$v=19$m=19456,t=2,p=1$AAAAAAAAAAAAAAAAAAAAAA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

pub(super) fn validate(password: &Secret) -> Result<(), AccountError> {
    let value = password.expose();
    if value.len() > RULES.max_utf8_bytes
        || !(RULES.min_codepoints..=RULES.max_codepoints).contains(&value.chars().count())
    {
        return Err(AccountError::InvalidPassword);
    }
    Ok(())
}

fn algorithm() -> Result<Argon2<'static>, AccountError> {
    let params = Params::new(MEMORY_KIB, ITERATIONS, LANES, Some(32))
        .map_err(|_| AccountError::Unavailable)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// This first storage format accepts exactly the production KDF profile. No PHC
/// supplied memory/iteration values can increase the per-task work bound.
fn bounded_phc(value: &str) -> Result<PasswordHash<'_>, AccountError> {
    if value.len() > 256 {
        return Err(AccountError::InvalidCredentials);
    }
    let hash = PasswordHash::new(value).map_err(|_| AccountError::InvalidCredentials)?;
    if hash.algorithm.as_str() != "argon2id"
        || hash.version != Some(19)
        || hash.params.iter().count() != 3
        || hash.params.get_decimal("m") != Some(MEMORY_KIB)
        || hash.params.get_decimal("t") != Some(ITERATIONS)
        || hash.params.get_decimal("p") != Some(LANES)
        || hash.hash.as_ref().map(|value| value.len()) != Some(32)
    {
        return Err(AccountError::InvalidCredentials);
    }
    let mut salt = [0u8; 64];
    let salt = hash
        .salt
        .ok_or(AccountError::InvalidCredentials)?
        .decode_b64(&mut salt)
        .map_err(|_| AccountError::InvalidCredentials)?;
    if salt.len() != 16 {
        return Err(AccountError::InvalidCredentials);
    }
    Ok(hash)
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, AccountError> + Send + 'static,
) -> Result<T, AccountError> {
    let permit = KDFS.try_acquire().map_err(|_| AccountError::Busy)?;
    tokio::task::spawn_blocking(move || {
        // Cancellation of the caller never releases the actual KDF budget.
        let _permit = permit;
        work()
    })
    .await
    .map_err(|_| AccountError::Unavailable)?
}

pub(super) async fn hash(password: Secret) -> Result<Zeroizing<String>, AccountError> {
    blocking(move || {
        let mut salt = Zeroizing::new([0u8; 16]);
        getrandom::fill(salt.as_mut()).map_err(|_| AccountError::Unavailable)?;
        let salt = SaltString::encode_b64(salt.as_ref()).map_err(|_| AccountError::Unavailable)?;
        Ok(Zeroizing::new(
            algorithm()?
                .hash_password(password.expose().as_bytes(), &salt)
                .map_err(|_| AccountError::Unavailable)?
                .to_string(),
        ))
    })
    .await
}

pub(super) async fn verify(
    password: Secret,
    candidate: Option<LoginCandidate>,
) -> Result<LoginCandidate, AccountError> {
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
            candidate.ok_or(AccountError::InvalidCredentials)
        } else {
            Err(AccountError::InvalidCredentials)
        }
    })
    .await
}

#[cfg(test)]
mod tests;
