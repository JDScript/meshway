use super::*;
use crate::{
    accounts::{Secret, one_time_digest},
    store::networks::management as store,
};
use meshway_model::JoinLinkId;
use meshway_model::{MembershipId, MembershipRole, Revision, Version};
use meshway_protocol::management::JoinLinkView;

pub struct CreatedLink {
    pub link: JoinLinkView,
    pub secret: Option<Secret>,
}
pub struct LinkInspection {
    pub link: JoinLinkView,
    pub network_name: String,
}
pub struct LinkPage {
    pub items: Vec<JoinLinkView>,
    pub next_cursor: Option<String>,
    pub as_of: DateTime<Utc>,
}
pub(crate) struct MemberRemoval {
    pub(crate) network: NetworkId,
    pub(crate) member: Option<MembershipId>,
    pub(crate) expected: Version,
    pub(crate) revision: Revision,
    pub(crate) request: RequestId,
}

impl Networks {
    pub async fn removal_impact(
        &self,
        token: &str,
        network: NetworkId,
        member: Option<MembershipId>,
        request: RequestId,
    ) -> Result<meshway_protocol::management::RemovalImpactResponse, NetworkError> {
        let digest = token_digest(token)?;
        let sealer = Arc::clone(&self.sealer);
        let cache = Arc::clone(&self.configuration_cache);
        self.handle
            .execute(move |db| {
                store::removal_impact(db, digest, network, member, request, &sealer, &cache)
            })
            .await?
    }
    pub async fn remove_member(
        &self,
        token: &str,
        network: NetworkId,
        member: Option<MembershipId>,
        expected: Version,
        revision: Revision,
        request: RequestId,
    ) -> Result<Revision, NetworkError> {
        let digest = token_digest(token)?;
        let sealer = Arc::clone(&self.sealer);
        let cache = Arc::clone(&self.configuration_cache);
        self.handle
            .execute(move |db| {
                store::remove_member(
                    db,
                    digest,
                    MemberRemoval {
                        network,
                        member,
                        expected,
                        revision,
                        request,
                    },
                    &sealer,
                    &cache,
                )
            })
            .await?
    }
    pub async fn change_role(
        &self,
        token: &str,
        network: NetworkId,
        member: MembershipId,
        expected: Version,
        role: MembershipRole,
        request: RequestId,
    ) -> Result<MemberSummary, NetworkError> {
        let digest = token_digest(token)?;
        self.handle
            .execute(move |db| {
                store::change_role(db, digest, network, member, expected, role, request)
            })
            .await?
    }
    pub async fn create_join_link(
        &self,
        token: &str,
        network: NetworkId,
        role: MembershipRole,
        ttl: u32,
        request: RequestId,
    ) -> Result<CreatedLink, NetworkError> {
        if !(60..=604800).contains(&ttl) {
            return Err(NetworkError::InvalidLimit);
        }
        let digest = token_digest(token)?;
        self.handle
            .execute(move |db| store::create_link(db, digest, network, role, ttl, request))
            .await?
    }
    pub async fn inspect_join_link(
        &self,
        token: &str,
        link: &str,
    ) -> Result<LinkInspection, NetworkError> {
        let digest = token_digest(token)?;
        let link = one_time_digest("mwj_", link).map_err(|_| NetworkError::InvalidLink)?;
        self.handle
            .execute(move |db| store::inspect_link(db, digest, link))
            .await?
    }
    pub async fn redeem_join_link(
        &self,
        token: &str,
        link: &str,
        request: RequestId,
    ) -> Result<(NetworkId, MembershipId), NetworkError> {
        let digest = token_digest(token)?;
        let link = one_time_digest("mwj_", link).map_err(|_| NetworkError::InvalidLink)?;
        self.handle
            .execute(move |db| store::redeem_link(db, digest, link, request))
            .await?
    }
    pub async fn revoke_join_link(
        &self,
        token: &str,
        network: NetworkId,
        id: JoinLinkId,
        request: RequestId,
    ) -> Result<(), NetworkError> {
        let digest = token_digest(token)?;
        self.handle
            .execute(move |db| store::revoke_link(db, digest, network, id, request))
            .await?
    }
    pub async fn join_links(
        &self,
        token: &str,
        network: NetworkId,
        limit: u16,
        cursor: Option<String>,
    ) -> Result<LinkPage, NetworkError> {
        pagination::validate_limit(limit)?;
        let digest = token_digest(token)?;
        let sealer = Arc::clone(&self.sealer);
        self.handle
            .execute(move |db| store::links(db, digest, network, limit, cursor, &sealer))
            .await?
    }
}
