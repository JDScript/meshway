use super::{Networks, pagination};
use crate::accounts::token_digest;
use crate::effect::Effect;
use crate::error::Error;
use crate::{
    accounts::{Secret, one_time_digest},
    store::networks::management as store,
};
use chrono::{DateTime, Utc};
use meshway_model::JoinLinkId;
use meshway_model::{MemberSummary, NetworkId, RequestId};
use meshway_model::{MembershipId, MembershipRole, Revision, Version};
use meshway_protocol::management::JoinLinkView;
use rusqlite::TransactionBehavior;
use std::sync::Arc;

pub struct CreatedLink {
    pub link: JoinLinkView,
    pub secret: Option<Secret>,
}
pub struct LinkInspection {
    pub link: JoinLinkView,
    pub network_name: String,
    pub already_member: bool,
}
pub struct LinkPage {
    pub total: meshway_model::Count,
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
    ) -> Result<meshway_protocol::management::RemovalImpactResponse, Error> {
        let digest = token_digest(token)?;
        let sealer = Arc::clone(&self.sealer);
        let cache = Arc::clone(&self.configuration_cache);
        self.handle
            .read(move |tx| {
                store::removal_impact(tx, digest, network, member, request, &sealer, &cache)
            })
            .await
    }
    pub async fn remove_member(
        &self,
        token: &str,
        network: NetworkId,
        member: Option<MembershipId>,
        expected: Version,
        revision: Revision,
        request: RequestId,
    ) -> Result<Revision, Error> {
        let digest = token_digest(token)?;
        let committer = crate::configuration::Committer::new(
            Arc::clone(&self.sealer),
            Arc::clone(&self.configuration_cache),
        );
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::remove_member(
                    tx,
                    digest,
                    MemberRemoval {
                        network,
                        member,
                        expected,
                        revision,
                        request,
                    },
                    &committer,
                )
            })
            .await
    }
    pub async fn change_role(
        &self,
        token: &str,
        network: NetworkId,
        member: MembershipId,
        expected: Version,
        role: MembershipRole,
        request: RequestId,
    ) -> Result<MemberSummary, Error> {
        let digest = token_digest(token)?;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                let (member, changed) =
                    store::change_role(tx, digest, network, member, expected, role, request)?;
                let effects = if changed {
                    vec![Effect::Authorization {
                        network_id: network,
                    }]
                } else {
                    Vec::new()
                };
                Ok((member, effects))
            })
            .await
    }
    pub async fn create_join_link(
        &self,
        token: &str,
        network: NetworkId,
        role: MembershipRole,
        ttl: u32,
        request: RequestId,
    ) -> Result<CreatedLink, Error> {
        if !(60..=604800).contains(&ttl) {
            return Err(Error::InvalidLimit);
        }
        let digest = token_digest(token)?;
        self.handle
            .write(move |tx| store::create_link(tx, digest, network, role, ttl, request))
            .await
    }
    pub async fn inspect_join_link(
        &self,
        token: &str,
        link: &str,
    ) -> Result<LinkInspection, Error> {
        let digest = token_digest(token)?;
        let link = one_time_digest("mwj_", link).map_err(|_| Error::InvalidLink)?;
        self.handle
            .read(move |tx| store::inspect_link(tx, digest, link))
            .await
    }
    pub async fn redeem_join_link(
        &self,
        token: &str,
        link: &str,
        request: RequestId,
    ) -> Result<(NetworkId, MembershipId), Error> {
        let digest = token_digest(token)?;
        let link = one_time_digest("mwj_", link).map_err(|_| Error::InvalidLink)?;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                let (result, changed) = store::redeem_link(tx, digest, link, request)?;
                let effects = if changed {
                    vec![Effect::Authorization {
                        network_id: result.0,
                    }]
                } else {
                    Vec::new()
                };
                Ok((result, effects))
            })
            .await
    }
    pub async fn revoke_join_link(
        &self,
        token: &str,
        network: NetworkId,
        id: JoinLinkId,
        request: RequestId,
    ) -> Result<(), Error> {
        let digest = token_digest(token)?;
        self.handle
            .write(move |tx| store::revoke_link(tx, digest, network, id, request))
            .await
    }
    pub async fn join_links(
        &self,
        token: &str,
        network: NetworkId,
        limit: u16,
        cursor: Option<String>,
    ) -> Result<LinkPage, Error> {
        pagination::validate_limit(limit)?;
        let digest = token_digest(token)?;
        let sealer = Arc::clone(&self.sealer);
        self.handle
            .read(move |tx| store::links(tx, digest, network, limit, cursor, &sealer))
            .await
    }
}
