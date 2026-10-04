use crate::error::Error;
use crate::secrets::{CursorScope, Sealer};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const TTL_MS: i64 = 15 * 60 * 1000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Window {
    pub(crate) upper: i64,
    pub(crate) memberships_upper: i64,
    pub(crate) last: i64,
    as_of: i64,
    expires: i64,
}

impl Window {
    pub(crate) fn new(upper: i64, memberships_upper: i64, now: i64) -> Result<Self, Error> {
        Ok(Self {
            upper,
            memberships_upper,
            last: 0,
            as_of: now,
            expires: now
                .checked_add(TTL_MS)
                .ok_or(Error::Internal("cursor expiry overflow"))?,
        })
    }
    pub(crate) fn as_of(&self) -> Result<DateTime<Utc>, Error> {
        DateTime::from_timestamp_millis(self.as_of).ok_or(Error::InvalidCursor)
    }
    pub(crate) fn decode(
        sealer: &Sealer,
        scope: CursorScope,
        cursor: &str,
        now: i64,
    ) -> Result<Self, Error> {
        if cursor.len() > 2048 {
            return Err(Error::InvalidCursor);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|_| Error::InvalidCursor)?;
        let plain = sealer
            .open_cursor(scope, &bytes)
            .map_err(|_| Error::InvalidCursor)?;
        let value: Self =
            serde_json::from_slice(plain.as_bytes()).map_err(|_| Error::InvalidCursor)?;
        if value.as_of < 0
            || value.as_of > now
            || value.expires <= now
            || value.expires.checked_sub(value.as_of) != Some(TTL_MS)
            || value.last < 0
            || value.upper < value.last
            || value.upper > meshway_model::MAX_SAFE_INTEGER as i64
            || value.memberships_upper < 0
            || value.memberships_upper > meshway_model::MAX_SAFE_INTEGER as i64
        {
            return Err(Error::InvalidCursor);
        }
        Ok(value)
    }
    pub(crate) fn encode(&self, sealer: &Sealer, scope: CursorScope) -> Result<String, Error> {
        let bytes = serde_json::to_vec(self).map_err(|_| Error::encode("pagination cursor"))?;
        let sealed = sealer.seal_cursor(scope, &bytes)?;
        Ok(URL_SAFE_NO_PAD.encode(sealed.as_bytes()))
    }
}

pub(crate) fn validate_limit(limit: u16) -> Result<(), Error> {
    if (1..=200).contains(&limit) {
        Ok(())
    } else {
        Err(Error::InvalidLimit)
    }
}
