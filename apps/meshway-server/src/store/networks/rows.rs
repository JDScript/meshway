use crate::error::Error;
use crate::networks::{MemberSummary, NetworkSummary};
use crate::store::authorization::parse_role;
use chrono::{DateTime, Utc};
use meshway_model::{MembershipId, MembershipRole, NetworkId, Revision, UserId, Version};
use rusqlite::Row;
use uuid::Uuid;

pub(super) struct RawNetwork {
    pub(super) sequence: i64,
    id: Vec<u8>,
    name: String,
    created_at: i64,
    revision: i64,
    membership: Vec<u8>,
    role: String,
}

impl RawNetwork {
    pub(super) fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            sequence: row.get(0)?,
            id: row.get(1)?,
            name: row.get(2)?,
            created_at: row.get(3)?,
            revision: row.get(4)?,
            membership: row.get(5)?,
            role: row.get(6)?,
        })
    }
    pub(super) fn into_view(self) -> Result<NetworkSummary, Error> {
        let role = parse_role(&self.role)?;
        Ok(NetworkSummary {
            id: network_id(&self.id)?,
            name: self.name,
            created_at: timestamp(self.created_at)?,
            current_revision: Revision::new(
                self.revision
                    .try_into()
                    .map_err(|_| Error::decode("networks.current_revision"))?,
            )
            .map_err(|_| Error::decode("networks.current_revision"))?,
            membership_id: MembershipId::new(uuid(&self.membership)?)
                .map_err(|_| Error::decode("memberships.id"))?,
            role,
            actions: crate::store::authorization::network_actions(role),
        })
    }
}

pub(super) struct RawMember {
    pub(super) sequence: i64,
    id: Vec<u8>,
    user: Vec<u8>,
    email: String,
    role: String,
    version: i64,
    joined_at: i64,
}

impl RawMember {
    pub(super) fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            sequence: row.get(0)?,
            id: row.get(1)?,
            user: row.get(2)?,
            email: row.get(3)?,
            role: row.get(4)?,
            version: row.get(5)?,
            joined_at: row.get(6)?,
        })
    }
    pub(super) fn into_view(
        self,
        caller: (MembershipId, MembershipRole),
        owner_count: i64,
    ) -> Result<MemberSummary, Error> {
        let id = MembershipId::new(uuid(&self.id)?).map_err(|_| Error::decode("memberships.id"))?;
        let role = parse_role(&self.role)?;
        Ok(MemberSummary {
            membership_id: id,
            user_id: UserId::new(uuid(&self.user)?)
                .map_err(|_| Error::decode("memberships.user_id"))?,
            email: self.email,
            role,
            version: Version::new(
                self.version
                    .try_into()
                    .map_err(|_| Error::decode("memberships.version"))?,
            )
            .map_err(|_| Error::decode("memberships.version"))?,
            joined_at: timestamp(self.joined_at)?,
            actions: crate::store::authorization::member_actions(caller, (id, role), owner_count),
        })
    }
}

fn uuid(bytes: &[u8]) -> Result<Uuid, Error> {
    Uuid::from_slice(bytes).map_err(|_| Error::decode("network row UUID"))
}
pub(super) fn network_id(bytes: &[u8]) -> Result<NetworkId, Error> {
    NetworkId::new(uuid(bytes)?).map_err(|_| Error::decode("networks.id"))
}
fn timestamp(value: i64) -> Result<DateTime<Utc>, Error> {
    DateTime::from_timestamp_millis(value).ok_or(Error::decode("network row timestamp"))
}
