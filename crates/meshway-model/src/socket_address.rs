use std::{fmt, net::SocketAddr, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;

/// A numeric socket address with a nonzero port and no IPv6 scope or flow info.
/// This observed fact alone does not establish reachability or grant authority.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, ToSchema)]
#[schema(value_type = String, format = "meshway-socket-address")]
pub struct SocketAddress(SocketAddr);

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("expected a numeric socket address with a nonzero port and no IPv6 scope or flow info")]
pub struct InvalidSocketAddress;

impl SocketAddress {
    pub fn new(address: SocketAddr) -> Result<Self, InvalidSocketAddress> {
        if address.port() == 0
            || matches!(address, SocketAddr::V6(value) if value.scope_id() != 0 || value.flowinfo() != 0)
        {
            return Err(InvalidSocketAddress);
        }
        Ok(Self(address))
    }

    pub const fn get(self) -> SocketAddr {
        self.0
    }
}

impl TryFrom<SocketAddr> for SocketAddress {
    type Error = InvalidSocketAddress;

    fn try_from(value: SocketAddr) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl FromStr for SocketAddress {
    type Err = InvalidSocketAddress;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value.parse().map_err(|_| InvalidSocketAddress)?)
    }
}

impl fmt::Display for SocketAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Serialize for SocketAddress {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SocketAddress {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
