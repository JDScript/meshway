//! Sealed listing cursors: the one envelope and lifetime of every paginated
//! read. A cursor is valid only within its `CursorScope` (sealing binds it)
//! and for 15 minutes after its listing's first page (every later cursor of
//! the listing keeps that start), and carries the listing's own body.
use crate::error::Error;
use crate::secrets::{CursorScope, Sealer};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Lifetime of every cursor.
pub(crate) const TTL_MS: i64 = 15 * 60 * 1000;
/// Longest cursor text accepted.
const MAX_TEXT: usize = 2048;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<T> {
    issued: i64,
    expires: i64,
    body: T,
}

/// Seal `body` for `scope`, for the listing whose first page was read at
/// `issued`.
pub(crate) fn seal<T: Serialize>(
    sealer: &Sealer,
    scope: CursorScope,
    body: T,
    issued: i64,
) -> Result<String, Error> {
    let envelope = Envelope {
        issued,
        expires: issued
            .checked_add(TTL_MS)
            .ok_or(Error::Internal("cursor expiry overflow"))?,
        body,
    };
    let bytes = serde_json::to_vec(&envelope).map_err(|_| Error::encode("cursor"))?;
    let sealed = sealer.seal_cursor(scope, &bytes)?;
    Ok(URL_SAFE_NO_PAD.encode(sealed.as_bytes()))
}

/// The body of a cursor sealed for `scope`, and when its listing began. Any
/// other scope, a tampered, oversized, future or expired cursor is
/// `InvalidCursor`.
pub(crate) fn open<T: DeserializeOwned>(
    sealer: &Sealer,
    scope: CursorScope,
    cursor: &str,
    now: i64,
) -> Result<(T, i64), Error> {
    if cursor.len() > MAX_TEXT {
        return Err(Error::InvalidCursor);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| Error::InvalidCursor)?;
    let plain = sealer
        .open_cursor(scope, &bytes)
        .map_err(|_| Error::InvalidCursor)?;
    let envelope: Envelope<T> =
        serde_json::from_slice(plain.as_bytes()).map_err(|_| Error::InvalidCursor)?;
    if envelope.issued < 0
        || envelope.issued > now
        || envelope.expires <= now
        || envelope.expires.checked_sub(envelope.issued) != Some(TTL_MS)
    {
        return Err(Error::InvalidCursor);
    }
    Ok((envelope.body, envelope.issued))
}
