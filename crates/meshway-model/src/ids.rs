use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;
use uuid::Uuid;

/// An ID must have exactly the canonical spelling of a nonzero UUID.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("expected a nonzero, lowercase, hyphenated UUID")]
pub struct InvalidId;

fn parse_uuid(value: &str) -> Result<Uuid, InvalidId> {
    let uuid = Uuid::parse_str(value).map_err(|_| InvalidId)?;
    if uuid.is_nil() || uuid.hyphenated().to_string() != value {
        return Err(InvalidId);
    }
    Ok(uuid)
}

macro_rules! id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        ///
        /// This is an identifier, not a credential. Generation and authorization
        /// belong to the caller. Parsing rejects aliases rather than normalizing them.
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, ToSchema)]
        #[schema(
            value_type = String,
            format = "uuid",
            min_length = 36,
            max_length = 36,
            pattern = "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
        )]
        pub struct $name(Uuid);

        impl $name {
            /// Wrap an existing UUID, rejecting the nil UUID.
            pub fn new(value: Uuid) -> Result<Self, InvalidId> {
                if value.is_nil() {
                    Err(InvalidId)
                } else {
                    Ok(Self(value))
                }
            }

            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            pub const fn into_uuid(self) -> Uuid {
                self.0
            }
        }

        impl TryFrom<Uuid> for $name {
            type Error = InvalidId;

            fn try_from(value: Uuid) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl FromStr for $name {
            type Err = InvalidId;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                parse_uuid(value).map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.hyphenated().fmt(formatter)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

id!(UserId, "Stable user identifier.");
id!(NetworkId, "Isolated network identifier.");
id!(
    NodeId,
    "Stable node identifier, independent of its addresses and keys."
);
id!(MembershipId, "Network membership identifier.");
id!(
    SessionId,
    "User session record identifier; not the session token."
);
id!(RequestId, "Request identifier.");
id!(
    UdpTraversalId,
    "Short-lived UDP traversal attempt identifier; not a credential."
);
id!(InstanceId, "Runtime instance identifier.");
id!(KeyId, "Key record identifier; not key material.");
id!(ActivityId, "Activity identifier.");
id!(ArtifactId, "Immutable dataset artifact identifier.");
id!(EventId, "Source observation event identifier.");
id!(ObservationId, "Leased observation task identifier.");
id!(
    ObservationSubscriptionId,
    "Independent observation demand and idempotency identifier."
);
id!(StateEpoch, "Volatile controller state epoch.");
id!(
    EnrollmentRequestId,
    "Device authorization request identifier, not a credential."
);
id!(
    EnrollmentKeyId,
    "One-use enrollment key identifier, not the secret."
);
id!(ChallengeId, "One-use proof challenge identifier.");
id!(
    JoinLinkId,
    "Network join-link record identifier; not the joining secret."
);
