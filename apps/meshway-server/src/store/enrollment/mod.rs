use crate::error::Error;
mod keys;
mod redeem;
mod requests;
use crate::{
    execution::proof,
    secrets::Sealer,
    store::{
        authorization::current_session,
        execution::{bytes, id, json, now, uuid},
    },
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
pub(crate) use keys::*;
use meshway_model::*;
use meshway_protocol::{WireSecret, enrollment::*};
pub(crate) use redeem::*;
pub(crate) use requests::*;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest as _, Sha256};
pub(crate) struct VerifiedEnrollmentProof {
    request: EnrollmentRequestId,
}

fn hash(s: &str) -> [u8; 32] {
    Sha256::digest(s.as_bytes()).into()
}
fn token() -> Result<String, Error> {
    let mut b = [0; 32];
    getrandom::fill(&mut b).map_err(|_| Error::ENTROPY)?;
    Ok(URL_SAFE_NO_PAD.encode(b))
}
fn pkce(challenge: &str, verifier: &str) -> Result<(), Error> {
    if !(43..=128).contains(&verifier.len())
        || !verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
        || URL_SAFE_NO_PAD.encode(hash(verifier)) != challenge
    {
        return Err(Error::InvalidCredentials);
    }
    Ok(())
}
fn manager(
    tx: &Transaction<'_>,
    digest: [u8; 32],
    network: NetworkId,
    now: i64,
) -> Result<(UserId, MembershipId), Error> {
    let user = current_session(tx, &digest, now)?.user.id;
    let (member,role):(Vec<u8>,String)=tx.query_row("SELECT m.id,m.role FROM memberships m JOIN networks n ON n.id=m.network_id WHERE m.network_id=?1 AND m.user_id=?2 AND m.revoked_at IS NULL AND n.deleted_at IS NULL",params![network.as_uuid().as_bytes().as_slice(),user.as_uuid().as_bytes().as_slice()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::NotFound)?;
    if role != "owner" && role != "admin" {
        return Err(Error::PermissionDenied);
    }
    Ok((user, id(member)?))
}
