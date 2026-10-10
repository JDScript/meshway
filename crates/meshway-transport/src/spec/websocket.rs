use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    str::FromStr,
};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, Visitor},
};
use utoipa::{
    PartialSchema, ToSchema,
    openapi::schema::{ObjectBuilder, Type},
};

use crate::spec::ValidationError;

/// Exact path, preserved without URL decoding or normalization.
/// Unicode is permitted, but whitespace, C0/C1 controls, query and fragment
/// delimiters are not. Percent escapes are not decoded by this value type.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash, ToSchema)]
#[serde(try_from = "String", into = "String")]
#[schema(value_type = String, pattern = r"^/[^\u0000-\u0020\u007F-\u009F\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000?#]*(?![\s\S])")]
pub struct WebSocketPath(String);

impl WebSocketPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for WebSocketPath {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if !value.starts_with('/')
            || value.chars().any(|character| {
                character.is_control()
                    || character.is_whitespace()
                    || matches!(character, '?' | '#')
            })
        {
            return Err(ValidationError::WebSocketPath);
        }
        Ok(Self(value))
    }
}

impl FromStr for WebSocketPath {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl From<WebSocketPath> for String {
    fn from(value: WebSocketPath) -> Self {
        value.0
    }
}

/// Case-preserving HTTP token for an additional WebSocket header.
/// Host, Connection, Upgrade, Content-Length, Transfer-Encoding, and every
/// Sec-WebSocket-* name are reserved, ignoring ASCII case. Handshake and HTTP
/// message framing fields belong to the upgrade implementation. No Unicode
/// header names are allowed by HTTP token syntax.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash, ToSchema)]
#[serde(try_from = "String", into = "String")]
#[schema(value_type = String, pattern = r"^(?!(?:[Hh][Oo][Ss][Tt]|[Cc][Oo][Nn][Nn][Ee][Cc][Tt][Ii][Oo][Nn]|[Uu][Pp][Gg][Rr][Aa][Dd][Ee]|[Cc][Oo][Nn][Tt][Ee][Nn][Tt]-[Ll][Ee][Nn][Gg][Tt][Hh]|[Tt][Rr][Aa][Nn][Ss][Ff][Ee][Rr]-[Ee][Nn][Cc][Oo][Dd][Ii][Nn][Gg])(?![\s\S]))(?![Ss][Ee][Cc]-[Ww][Ee][Bb][Ss][Oo][Cc][Kk][Ee][Tt]-)[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\s\S])")]
pub struct WebSocketHeaderName(String);

impl WebSocketHeaderName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for WebSocketHeaderName {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        {
            return Err(ValidationError::HeaderName);
        }
        let lowercase = value.to_ascii_lowercase();
        if matches!(
            lowercase.as_str(),
            "host" | "connection" | "upgrade" | "content-length" | "transfer-encoding"
        ) || lowercase.starts_with("sec-websocket-")
        {
            return Err(ValidationError::ReservedHeader);
        }
        Ok(Self(value))
    }
}

impl FromStr for WebSocketHeaderName {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl From<WebSocketHeaderName> for String {
    fn from(value: WebSocketHeaderName) -> Self {
        value.0
    }
}

/// Header text preserved verbatim, including empty values and horizontal tabs.
/// CR/LF, NUL and other Unicode control characters (except HTAB) are rejected.
/// Unicode text is represented as UTF-8; protocol-specific semantic constraints
/// and request/header size limits remain the consumer's responsibility.
// Debug output is redacted; serialization and explicit access preserve secrets.
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash, ToSchema)]
#[serde(try_from = "String", into = "String")]
#[schema(value_type = String, pattern = r"^[^\u0000-\u0008\u000A-\u001F\u007F-\u009F]*(?![\s\S])")]
pub struct WebSocketHeaderValue(String);

impl fmt::Debug for WebSocketHeaderValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl WebSocketHeaderValue {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for WebSocketHeaderValue {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value
            .chars()
            .any(|character| character.is_control() && character != '\t')
        {
            return Err(ValidationError::HeaderValue);
        }
        Ok(Self(value))
    }
}

impl FromStr for WebSocketHeaderValue {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl From<WebSocketHeaderValue> for String {
    fn from(value: WebSocketHeaderValue) -> Self {
        value.0
    }
}

/// Additional request or response headers; original name casing is retained.
/// All names/values are validated, and repeated names (including differently
/// cased spellings) fail rather than overwriting. Case-insensitive uniqueness
/// is a semantic constraint beyond JSON Schema's property-name validation.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct WebSocketHeaders(BTreeMap<WebSocketHeaderName, WebSocketHeaderValue>);

impl WebSocketHeaders {
    pub fn try_from_iter<I>(headers: I) -> Result<Self, ValidationError>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let mut values = BTreeMap::new();
        let mut names = BTreeSet::new();
        for (name, value) in headers {
            let name = WebSocketHeaderName::try_from(name)?;
            let value = WebSocketHeaderValue::try_from(value)?;
            if !names.insert(name.as_str().to_ascii_lowercase()) {
                return Err(ValidationError::DuplicateHeader);
            }
            values.insert(name, value);
        }
        Ok(Self(values))
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl TryFrom<BTreeMap<String, String>> for WebSocketHeaders {
    type Error = ValidationError;

    fn try_from(value: BTreeMap<String, String>) -> Result<Self, Self::Error> {
        Self::try_from_iter(value)
    }
}

impl<'de> Deserialize<'de> for WebSocketHeaders {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct HeadersVisitor;

        impl<'de> Visitor<'de> for HeadersVisitor {
            type Value = WebSocketHeaders;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object of unique additional WebSocket headers")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                // Do not deserialize through a map: it would silently discard
                // identical duplicate JSON keys before they can be rejected.
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, String>()? {
                    entries.push(entry);
                }
                WebSocketHeaders::try_from_iter(entries).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_map(HeadersVisitor)
    }
}

impl PartialSchema for WebSocketHeaders {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        ObjectBuilder::new()
            .schema_type(Type::Object)
            .property_names(Some(WebSocketHeaderName::schema()))
            .additional_properties(Some(WebSocketHeaderValue::schema()))
            .description(Some("Additional WebSocket headers, preserving casing. HTTP token names cannot override Host, Connection, Upgrade, Content-Length, Transfer-Encoding or Sec-WebSocket-*, ignoring ASCII case. Handshake and HTTP message framing fields belong to the upgrade implementation. Names must also be unique ignoring ASCII case (semantic validation)."))
            .into()
    }
}

impl ToSchema for WebSocketHeaders {}
