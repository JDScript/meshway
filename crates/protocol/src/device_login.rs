//! Native browser authorization, not a generic OAuth server.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const DEVICE_CALLBACK_URI: &str = "meshway://enroll/callback";

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AuthorizeDeviceRequest {
    #[schema(
        pattern = "^meshway://enroll/callback$",
        min_length = 25,
        max_length = 25
    )]
    pub redirect_uri: String,
    #[schema(
        min_length = 43,
        max_length = 43,
        pattern = "^[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$"
    )]
    pub code_challenge: String,
    #[schema(
        min_length = 1,
        max_length = 128,
        pattern = r"[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]"
    )]
    pub name: String,
    #[schema(
        min_length = 1,
        max_length = 64,
        pattern = r"[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]"
    )]
    pub agent_version: String,
}

impl AuthorizeDeviceRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.redirect_uri != DEVICE_CALLBACK_URI {
            return Err("invalid device redirect URI");
        }
        let bytes = self.code_challenge.as_bytes();
        if bytes.len() != 43
            || !bytes
                .iter()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(c))
            || !b"AEIMQUYcgkosw048".contains(&bytes[42])
        {
            return Err("invalid S256 challenge");
        }
        super::validate_text(&self.name, 128)?;
        super::validate_text(&self.agent_version, 64)
    }
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AuthorizeDeviceResponse {
    pub code: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExchangeDeviceRequest {
    #[schema(
        min_length = 43,
        max_length = 128,
        pattern = "^[A-Za-z0-9._~-]{43,128}$"
    )]
    pub code_verifier: String,
    #[schema(
        pattern = "^meshway://enroll/callback$",
        min_length = 25,
        max_length = 25
    )]
    pub redirect_uri: String,
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub node_credential: String,
    #[schema(
        min_length = 1,
        max_length = 128,
        pattern = r"[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]"
    )]
    pub name: String,
    #[schema(
        min_length = 1,
        max_length = 64,
        pattern = r"[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]"
    )]
    pub agent_version: String,
}

impl ExchangeDeviceRequest {
    pub fn enrollment(&self) -> Result<super::EnrollAgentRequest, &'static str> {
        if !(43..=128).contains(&self.code_verifier.len())
            || !self
                .code_verifier
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._~-".contains(&c))
            || self.redirect_uri != DEVICE_CALLBACK_URI
        {
            return Err("invalid device exchange binding");
        }
        let request = super::EnrollAgentRequest {
            node_credential: self.node_credential.clone(),
            name: self.name.clone(),
            agent_version: self.agent_version.clone(),
        };
        request.validate()?;
        Ok(request)
    }
}

/// RFC 7636 S256: SHA-256 encoded as unpadded base64url.
pub fn s256(verifier: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let bytes = super::local::digest(verifier);
    let mut output = String::with_capacity(43);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        output.push(char::from(ALPHABET[usize::from(a >> 2)]));
        output.push(char::from(ALPHABET[usize::from(((a & 3) << 4) | (b >> 4))]));
        if chunk.len() > 1 {
            output.push(char::from(
                ALPHABET[usize::from(((b & 15) << 2) | (c >> 6))],
            ));
        }
        if chunk.len() > 2 {
            output.push(char::from(ALPHABET[usize::from(c & 63)]));
        }
    }
    output
}

#[cfg(test)]
mod tests {
    #[test]
    fn rfc7636_vector() {
        assert_eq!(
            super::s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
