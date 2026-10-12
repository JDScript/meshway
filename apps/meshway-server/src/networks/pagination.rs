use crate::cursor;
use crate::error::Error;
use crate::secrets::{CursorScope, Sealer};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The snapshot one history listing pages through.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    upper: i64,
    memberships_upper: i64,
    last: i64,
}

pub(crate) struct Window {
    pub(crate) upper: i64,
    pub(crate) memberships_upper: i64,
    pub(crate) last: i64,
    as_of: i64,
}

impl Window {
    pub(crate) fn new(upper: i64, memberships_upper: i64, now: i64) -> Result<Self, Error> {
        now.checked_add(cursor::TTL_MS)
            .ok_or(Error::Internal("cursor expiry overflow"))?;
        Ok(Self {
            upper,
            memberships_upper,
            last: 0,
            as_of: now,
        })
    }
    pub(crate) fn as_of(&self) -> Result<DateTime<Utc>, Error> {
        DateTime::from_timestamp_millis(self.as_of).ok_or(Error::InvalidCursor)
    }
    pub(crate) fn decode(
        sealer: &Sealer,
        scope: CursorScope,
        text: &str,
        now: i64,
    ) -> Result<Self, Error> {
        let (body, as_of): (Body, i64) = cursor::open(sealer, scope, text, now)?;
        if body.last < 0
            || body.upper < body.last
            || body.upper > meshway_model::MAX_SAFE_INTEGER as i64
            || body.memberships_upper < 0
            || body.memberships_upper > meshway_model::MAX_SAFE_INTEGER as i64
        {
            return Err(Error::InvalidCursor);
        }
        Ok(Self {
            upper: body.upper,
            memberships_upper: body.memberships_upper,
            last: body.last,
            as_of,
        })
    }
    pub(crate) fn encode(&self, sealer: &Sealer, scope: CursorScope) -> Result<String, Error> {
        let body = Body {
            upper: self.upper,
            memberships_upper: self.memberships_upper,
            last: self.last,
        };
        cursor::seal(sealer, scope, body, self.as_of)
    }
}

pub(crate) fn validate_limit(limit: u16) -> Result<(), Error> {
    if (1..=200).contains(&limit) {
        Ok(())
    } else {
        Err(Error::InvalidLimit)
    }
}
