//! Real SQLite/account authorization and HTTP SSE transport tests. The narrow
//! provider is intentionally test-only; no production fallback authorizes Nodes.
use super::service::*;
use super::*;
use crate::effect::{Effect, EffectSink};
use crate::{
    App, Settings, http,
    secrets::{Purpose, Sealer},
};
use meshway_configuration::NetworkConfiguration;
use meshway_model::{MembershipId, MembershipRole};
use meshway_protocol::{NetworkSummary, events::StreamEvent};
use rusqlite::{OptionalExtension, Transaction, params};
use std::{sync::Arc, time::Duration as StdDuration};
use tokio::sync::oneshot;

fn rid() -> meshway_model::RequestId {
    meshway_model::RequestId::new(crate::accounts::random_uuid().unwrap()).unwrap()
}

struct SqlProvider {
    sealer: Arc<Sealer>,
}
impl StateProvider for SqlProvider {
    type Configuration = NetworkConfiguration;
    type Entities = NetworkSummary;
    type Progress = Sequence;
    fn view(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<ViewContext<Self::Configuration, Self::Entities, Self::Progress>, Error> {
        let ReadIdentity::UserSession { digest, network } = identity else {
            return Err(Error::InvalidSession);
        };
        let session:Option<(Vec<u8>,i64)>=tx.query_row("SELECT u.id,s.expires_at FROM user_sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=?1 AND s.revoked_at IS NULL AND s.expires_at>?2 AND s.credential_version=u.credential_version AND u.disabled_at IS NULL",params![digest.as_slice(),now.timestamp_millis()],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let (user, expires) = session.ok_or(Error::InvalidSession)?;
        let (name,created,revision,membership,role,config):(String,i64,i64,Vec<u8>,String,Vec<u8>)=tx.query_row("SELECT n.name,n.created_at,n.current_revision,m.id,m.role,c.configuration FROM networks n JOIN memberships m ON m.network_id=n.id JOIN configuration_versions c ON c.network_id=n.id AND c.revision=n.current_revision WHERE n.id=?1 AND m.user_id=?2 AND m.revoked_at IS NULL",params![network.as_uuid().as_bytes().as_slice(),user],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).optional()?.ok_or(Error::NotFound)?;
        let revision = Revision::new(
            revision
                .try_into()
                .map_err(|_| Error::decode("networks.current_revision"))?,
        )
        .map_err(|_| Error::decode("networks.current_revision"))?;
        let role = match role.as_str() {
            "owner" => MembershipRole::Owner,
            "admin" => MembershipRole::Admin,
            "member" => MembershipRole::Member,
            _ => return Err(Error::decode("memberships.role")),
        };
        let membership_id =
            MembershipId::new(uuid::Uuid::from_slice(&membership).unwrap()).unwrap();
        let plaintext = self
            .sealer
            .open(*network, revision, Purpose::Configuration, &config)?;
        let configuration = meshway_configuration::restore_canonical_json(
            std::str::from_utf8(plaintext.as_bytes()).unwrap(),
        )
        .map_err(|_| Error::decode("configuration_versions.configuration"))?;
        let mut authorization = membership;
        authorization.extend_from_slice(format!("{role:?}").as_bytes());
        let scope = VisibilityScope {
            network_id: *network,
            configuration_revision: revision,
            authorization_digest: crate::resources::content_digest(&authorization),
            observation_authorization_digest: crate::resources::content_digest(&authorization),
            visible: configuration.nodes.keys().copied().collect(),
            hostnames: configuration
                .nodes
                .iter()
                .map(|(id, node)| (*id, node.hostname.clone()))
                .collect(),
            full: configuration.nodes.keys().copied().collect(),
            endpoint_resources: None,
            observation_paths: Default::default(),
            mesh_addresses: Default::default(),
            expires_at: DateTime::from_timestamp_millis(expires).unwrap(),
            current: BTreeMap::new(),
        };
        Ok(ViewContext {
            scope,
            configuration,
            entities: NetworkSummary {
                id: *network,
                name,
                created_at: DateTime::from_timestamp_millis(created).unwrap(),
                current_revision: revision,
                membership_id,
                role,
                actions: crate::store::authorization::network_actions(role),
            },
            progress: None,
            member_ids: std::collections::BTreeSet::from([membership_id]),
        })
    }
    fn report(
        &self,
        _: &Transaction<'_>,
        _: &str,
        _: DateTime<Utc>,
    ) -> Result<ReportAdmission, Error> {
        Err(Error::InvalidSession)
    }
}
type TestEvent = StreamEvent<NetworkConfiguration, NetworkSummary, Sequence>;

struct Frames {
    response: reqwest::Response,
    buffer: Vec<u8>,
}
impl Frames {
    async fn next(&mut self) -> Option<(Option<String>, TestEvent)> {
        tokio::time::timeout(StdDuration::from_secs(5), async {
            loop {
                if let Some(index) = self.buffer.windows(2).position(|pair| pair == b"\n\n") {
                    let bytes: Vec<_> = self.buffer.drain(..index + 2).collect();
                    let text = std::str::from_utf8(&bytes).unwrap();
                    let id = text
                        .lines()
                        .find_map(|line| line.strip_prefix("id: "))
                        .map(str::to_owned);
                    let data = text
                        .lines()
                        .filter_map(|line| line.strip_prefix("data: "))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if data.is_empty() {
                        continue;
                    }
                    return Some((id, serde_json::from_str(&data).unwrap()));
                }
                let chunk = self.response.chunk().await.unwrap()?;
                // Deliberately consume arbitrary byte fragments, including
                // UTF-8 boundaries; framing never assumes one HTTP chunk/frame.
                for piece in chunk.chunks(3) {
                    self.buffer.extend_from_slice(piece);
                }
            }
        })
        .await
        .expect("stream must make bounded progress")
    }
}
async fn open(client: &reqwest::Client, url: &str, token: &str, cursor: Option<&str>) -> Frames {
    let mut request = client.get(url).bearer_auth(token);
    if let Some(cursor) = cursor {
        request = request.header("Last-Event-ID", cursor);
    }
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    Frames {
        response,
        buffer: vec![],
    }
}


mod streams;
