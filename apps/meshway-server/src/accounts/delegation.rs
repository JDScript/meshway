//! Delegated sign-in for configured browser clients (OAuth 2.0 authorization
//! code with PKCE). A signed-in user authorizes a client and receives a
//! one-time code bound to the authorizing session, the client, its redirect
//! target and a code challenge; the client exchanges code and verifier for a
//! short-lived session that acts as the user but cannot manage the account.
use super::{Secret, issue_one_time, one_time_digest};
use crate::error::Error;
use crate::settings::Url;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::time::Duration;

pub const CODE_PREFIX: &str = "mwd_";
pub const CODE_TTL: Duration = Duration::from_secs(60);
pub const SESSION_TTL: Duration = Duration::from_secs(60 * 60);

/// Browser clients a deployment allows to obtain delegated sessions: a short
/// name recorded on the session and the origin its redirect targets and
/// cross-origin requests must come from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DelegationClients {
    clients: BTreeMap<String, Url>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidDelegationClient {
    #[error("expected <name>=<origin> with a name of 1-32 lowercase letters, digits or hyphens")]
    Name,
    #[error("{0}")]
    Origin(#[from] crate::settings::InvalidConsoleOrigin),
    #[error("duplicate delegation client name")]
    Duplicate,
}

impl DelegationClients {
    pub fn parse<'a>(
        entries: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, InvalidDelegationClient> {
        let mut clients = BTreeMap::new();
        for entry in entries {
            let (name, origin) = entry.split_once('=').ok_or(InvalidDelegationClient::Name)?;
            if !valid_client_name(name) {
                return Err(InvalidDelegationClient::Name);
            }
            let origin = crate::settings::console_origin(origin)?;
            if clients.insert(name.to_owned(), origin).is_some() {
                return Err(InvalidDelegationClient::Duplicate);
            }
        }
        Ok(Self { clients })
    }
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
    pub fn origin(&self, client: &str) -> Option<&Url> {
        self.clients.get(client)
    }
    /// Origins allowed for cross-origin browser requests.
    pub fn origins(&self) -> impl Iterator<Item = &Url> {
        self.clients.values()
    }
    /// A redirect target is accepted when it is an absolute `http(s)` URL on
    /// the client's origin without credentials or fragment.
    pub fn redirect_target(&self, client: &str, redirect_uri: &str) -> Result<Url, Error> {
        let origin = self.origin(client).ok_or(Error::InvalidRequest)?;
        if redirect_uri.len() > 2048 {
            return Err(Error::InvalidRequest);
        }
        let target = Url::parse(redirect_uri).map_err(|_| Error::InvalidRequest)?;
        if target.origin() != origin.origin()
            || !target.username().is_empty()
            || target.password().is_some()
            || target.fragment().is_some()
        {
            return Err(Error::InvalidRequest);
        }
        Ok(target)
    }
}

fn valid_client_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// RFC 7636 `code_challenge`: base64url without padding of SHA-256, 43 chars.
pub fn valid_code_challenge(value: &str) -> bool {
    value.len() == 43
        && URL_SAFE_NO_PAD
            .decode(value)
            .is_ok_and(|bytes| bytes.len() == 32)
}

/// RFC 7636 `code_verifier`: 43-128 unreserved characters.
pub fn valid_code_verifier(value: &str) -> bool {
    (43..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

pub fn challenge_of(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub(crate) fn issue_code() -> Result<(Secret, [u8; 32]), Error> {
    issue_one_time(CODE_PREFIX)
}

pub(crate) fn code_digest(code: &str) -> Result<[u8; 32], Error> {
    one_time_digest(CODE_PREFIX, code).map_err(|_| Error::InvalidDelegation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clients_require_names_and_secure_origins() {
        let clients =
            DelegationClients::parse(["site=https://meshway.example", "dev=http://localhost:3000"])
                .unwrap();
        assert_eq!(clients.origins().count(), 2);
        for bad in [
            "https://meshway.example",
            "Site=https://meshway.example",
            "site=http://meshway.example",
            "a-very-long-client-name-over-thirty-two=https://x.example",
        ] {
            assert!(DelegationClients::parse([bad]).is_err(), "{bad}");
        }
        assert!(
            DelegationClients::parse(["site=https://a.example", "site=https://b.example"]).is_err()
        );
        assert!(
            clients
                .redirect_target("site", "https://meshway.example/docs/callback?x=1")
                .is_ok()
        );
        for bad in [
            ("site", "https://other.example/callback"),
            ("site", "https://meshway.example/callback#frag"),
            ("site", "https://user@meshway.example/callback"),
            ("nope", "https://meshway.example/callback"),
            ("dev", "http://localhost:3001/callback"),
        ] {
            assert!(clients.redirect_target(bad.0, bad.1).is_err(), "{bad:?}");
        }
        assert_eq!(
            clients
                .redirect_target("site", "https://meshway.example/cb?keep=1")
                .unwrap()
                .as_str(),
            "https://meshway.example/cb?keep=1"
        );
    }

    #[test]
    fn pkce_values_follow_rfc_7636() {
        let verifier = "a".repeat(43);
        assert!(valid_code_verifier(&verifier));
        assert!(!valid_code_verifier(&"a".repeat(42)));
        assert!(!valid_code_verifier(&"a".repeat(129)));
        assert!(!valid_code_verifier(&format!("{}/", "a".repeat(43))));
        let challenge = challenge_of(&verifier);
        assert!(valid_code_challenge(&challenge));
        assert!(!valid_code_challenge("short"));
        assert!(!valid_code_challenge(&format!("{}=", &challenge[..42])));
    }
}
