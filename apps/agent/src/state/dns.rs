//! DNS sync uses the existing identity and generated HTTP contract. The private
//! cache is a replay/binding floor only; startup never publishes it.

use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};

use hickory_proto::rr::Name;
use meshway_protocol::{AgentDnsSnapshot, MAX_DNS_BYTES, dns_digest};
use serde::{Deserialize, Serialize};
use tokio::time::{Instant, sleep};

use super::{Agent, AgentError, Identity, local};
use crate::{
    generated::client::HttpClient,
    managed_dns::{Guard, Server, Zone},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    format_version: u32,
    controller: String,
    node_id: String,
    credential_hash: Vec<u8>,
    snapshot: AgentDnsSnapshot,
}

fn path(state: &Path) -> PathBuf {
    let mut value = state.as_os_str().to_owned();
    value.push(".dns.json");
    PathBuf::from(value)
}

fn validate(
    snapshot: &AgentDnsSnapshot,
    node_id: &str,
    previous: Option<&AgentDnsSnapshot>,
) -> Result<Zone, AgentError> {
    let payload = &snapshot.payload;
    payload
        .validate()
        .map_err(|_| AgentError::Invalid("invalid DNS schema, records or configuration"))?;
    if payload.node_id != node_id
        || dns_digest(payload).map_err(|_| AgentError::Invalid("cannot hash DNS snapshot"))?
            != snapshot.digest
        || serde_json::to_vec(snapshot)
            .map_err(|_| AgentError::Invalid("cannot encode DNS snapshot"))?
            .len()
            > MAX_DNS_BYTES
    {
        return Err(AgentError::Invalid("invalid DNS digest or node binding"));
    }
    if let Some(old) = previous {
        if payload.revision < old.payload.revision
            || (payload.revision == old.payload.revision && snapshot.digest != old.digest)
            || payload.network_id != old.payload.network_id
        {
            return Err(AgentError::Invalid(
                "DNS rollback, same-revision mutation or network rebinding",
            ));
        }
        for record in &payload.records {
            if old.payload.records.iter().any(|old| {
                old.node_id == record.node_id
                    && (old.mesh_ipv4 != record.mesh_ipv4 || old.mesh_ipv6 != record.mesh_ipv6)
            }) {
                return Err(AgentError::Invalid("DNS assigned identity address changed"));
            }
        }
    }
    let parse_name = |value: &str| {
        Name::from_ascii(format!("{value}.")).map_err(|_| AgentError::Invalid("invalid DNS name"))
    };
    let mut zone = Zone {
        name: parse_name(&payload.dns.zone)?,
        ttl: payload.dns.ttl_seconds,
        forward: Default::default(),
        reverse: Default::default(),
    };
    for record in &payload.records {
        let v4: Ipv4Addr = record
            .mesh_ipv4
            .parse()
            .map_err(|_| AgentError::Invalid("invalid DNS IPv4"))?;
        let v6: Ipv6Addr = record
            .mesh_ipv6
            .parse()
            .map_err(|_| AgentError::Invalid("invalid DNS IPv6"))?;
        if v4.to_string() != record.mesh_ipv4 || v6.to_string() != record.mesh_ipv6 {
            return Err(AgentError::Invalid("noncanonical DNS assigned address"));
        }
        let canonical = parse_name(&record.canonical_name)?;
        for owner in std::iter::once(&record.canonical_name).chain(&record.aliases) {
            let name = parse_name(owner)?;
            if !zone.name.zone_of(&name) || zone.forward.insert(name, (v4, v6)).is_some() {
                return Err(AgentError::Invalid("DNS name collision or zone escape"));
            }
        }
        zone.reverse.insert(Name::from(v4), canonical.clone());
        zone.reverse.insert(Name::from(v6), canonical);
    }
    Ok(zone)
}

fn load(state: &Path, identity: &Identity) -> Result<Option<Cache>, AgentError> {
    let file = path(state);
    match std::fs::symlink_metadata(&file) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let cache: Cache = serde_json::from_str(&local::read_private_bounded(
        &file,
        MAX_DNS_BYTES + 64 * 1024,
    )?)
    .map_err(|_| AgentError::Invalid("invalid DNS cache; refusing to overwrite"))?;
    if cache.format_version != 1
        || cache.controller != identity.controller
        || Some(&cache.node_id) != identity.node_id.as_ref()
        || cache.credential_hash != local::digest(&identity.node_credential)
    {
        return Err(AgentError::Invalid(
            "DNS cache belongs to another controller or credential",
        ));
    }
    validate(&cache.snapshot, &cache.node_id, None)?;
    Ok(Some(cache))
}

/// Owned DNS supervision; callers must race this with the *same* Agent run
/// future. A service failure is fatal and cancellation closes all DNS sockets.
pub struct DnsService {
    server: Server,
    sync: Sync,
}

struct Sync {
    state: PathBuf,
    identity: Identity,
    cache: Option<Cache>,
    guard: Guard,
    listen: SocketAddr,
    interval: Duration,
    lease: Duration,
}

impl Agent {
    pub async fn dns_service(
        &mut self,
        listen: SocketAddr,
        interval: Duration,
        lease: Duration,
    ) -> Result<DnsService, AgentError> {
        if !(Duration::from_secs(10)..=Duration::from_secs(300)).contains(&lease)
            || interval.is_zero()
            || interval >= lease
        {
            return Err(AgentError::Invalid(
                "DNS lease must be 10–300 seconds and exceed the polling interval",
            ));
        }
        if self.identity.node_id.is_none() {
            return Err(AgentError::Invalid(
                "DNS publication requires an enrolled node identity",
            ));
        }
        // Reload privately instead of exposing/copying credentials to CLI code.
        let identity = Identity::load(&self.path)?;
        if identity != self.identity {
            return Err(AgentError::Invalid("identity changed before DNS startup"));
        }
        let cache = load(&self.path, &identity)?;
        let guard = Guard::default();
        if let Some(cache) = &cache {
            guard.observe_revision(cache.snapshot.payload.revision);
        }
        if let Some(cache) = &self.cache {
            guard.observe_revision(cache.public_inventory().revision);
        }
        let server = Server::bind(listen).await?;
        self.dns_guard = Some(guard.clone());
        Ok(DnsService {
            server,
            sync: Sync {
                state: self.path.clone(),
                identity,
                cache,
                guard,
                listen,
                interval,
                lease,
            },
        })
    }
}

impl DnsService {
    pub async fn run(self) -> Result<(), AgentError> {
        let guard = self.sync.guard.clone();
        let listen = self.sync.listen;
        let expiry = async {
            let mut timer = tokio::time::interval(Duration::from_millis(250));
            loop {
                timer.tick().await;
                if guard.expire(Instant::now()) {
                    println!(
                        "{}",
                        serde_json::json!({"event": "managed_dns", "state": "expired", "listen": listen})
                    );
                }
            }
        };
        tokio::select! {
            result = self.server.serve(guard.clone()) => result,
            result = self.sync.run() => result,
            _ = expiry => unreachable!("DNS expiry watchdog runs until cancellation"),
        }
    }
}

impl Sync {
    fn event(&self, state: &'static str, snapshot: Option<&AgentDnsSnapshot>) {
        #[derive(Serialize)]
        struct Event<'a> {
            event: &'static str,
            state: &'static str,
            listen: SocketAddr,
            revision: Option<u32>,
            digest: Option<&'a str>,
            zone: Option<&'a str>,
        }
        let event = Event {
            event: "managed_dns",
            state,
            listen: self.listen,
            revision: snapshot.map(|s| s.payload.revision),
            digest: snapshot.map(|s| s.digest.as_str()),
            zone: snapshot.map(|s| s.payload.dns.zone.as_str()),
        };
        if let Ok(event) = serde_json::to_string(&event) {
            println!("{event}");
        }
    }

    async fn run(mut self) -> Result<(), AgentError> {
        self.event("waiting_for_fresh_snapshot", None);
        let mut backoff = 1;
        loop {
            let result = self.tick().await;
            let delay = match result {
                Ok(()) => {
                    backoff = 1;
                    self.interval
                }
                Err(error) if error.retryable() => {
                    let delay = Duration::from_secs(backoff);
                    backoff = (backoff * 2).min(30);
                    eprintln!("DNS {error}; retrying in {}s", delay.as_secs());
                    delay
                }
                Err(error) => {
                    self.guard.withdraw();
                    self.event("failed", None);
                    return Err(error);
                }
            };
            sleep(delay).await;
        }
    }

    async fn tick(&mut self) -> Result<(), AgentError> {
        let result = self.pull().await;
        if let Err(error) = &result
            && (!error.retryable() || matches!(error, AgentError::Http { status: 409, .. }))
        {
            self.guard.withdraw();
        }
        result
    }

    async fn pull(&mut self) -> Result<(), AgentError> {
        let client = HttpClient::new()
            .with_base_url(&self.identity.controller)
            .with_api_key(&self.identity.node_credential)
            .with_max_response_body_bytes(MAX_DNS_BYTES);
        let response = client.get_agent_dns().await;
        let snapshot: AgentDnsSnapshot = match response {
            Ok(value) => match super::connections::convert(value) {
                Ok(value) => value,
                Err(error) => {
                    self.guard.withdraw();
                    return Err(error);
                }
            },
            Err(error) => {
                let withdraw = error
                    .api()
                    .is_some_and(|error| matches!(error.status, 200..=299 | 401 | 403 | 409));
                let error = AgentError::from(error);
                if withdraw || !error.retryable() {
                    self.guard.withdraw();
                }
                return Err(error);
            }
        };
        let fresh_at = Instant::now();
        let changed =
            self.cache.as_ref().map(|cache| &cache.snapshot.digest) != Some(&snapshot.digest);
        // A replacement withdraws before validation or disk IO. Identical fresh
        // reads keep the still-valid old lease while checking durable storage;
        // any validation/persistence failure is synchronously withdrawn by tick.
        if changed {
            self.guard.withdraw();
        }
        let zone = validate(
            &snapshot,
            self.identity.node_id.as_deref().expect("enrolled DNS"),
            self.cache.as_ref().map(|c| &c.snapshot),
        )?;
        // The separately validated configuration cache pins this node's assigned
        // addresses even across a disabled (empty-record) DNS generation.
        if let Some(configuration) = super::configuration::load(&self.state, &self.identity)? {
            let inventory = configuration.public_inventory();
            let own = &inventory.node;
            if snapshot.payload.records.iter().any(|record| {
                record.node_id == own.id
                    && (record.mesh_ipv4 != own.mesh_ipv4 || record.mesh_ipv6 != own.mesh_ipv6)
            }) {
                return Err(AgentError::Invalid(
                    "DNS self address differs from assigned configuration",
                ));
            }
            if inventory.revision == snapshot.payload.revision {
                let nodes: Vec<_> = std::iter::once(own.clone())
                    .chain(inventory.peers.iter().cloned())
                    .collect();
                let (expected, _) =
                    meshway_protocol::compile_dns_records(&snapshot.payload.dns, &nodes).map_err(
                        |_| AgentError::Invalid("cannot validate DNS inventory projection"),
                    )?;
                let expected = serde_json::to_vec(&expected)
                    .map_err(|_| AgentError::Invalid("cannot encode expected DNS inventory"))?;
                let actual = serde_json::to_vec(&snapshot.payload.records)
                    .map_err(|_| AgentError::Invalid("cannot encode DNS inventory"))?;
                if expected != actual {
                    return Err(AgentError::Invalid(
                        "DNS names or addresses differ from same-revision inventory",
                    ));
                }
            }
        }
        if snapshot.payload.revision < self.guard.revision_floor() {
            return Err(AgentError::Http {
                status: 409,
                retryable: true,
            });
        }
        self.guard.observe_revision(snapshot.payload.revision);
        if Identity::load(&self.state)? != self.identity
            || load(&self.state, &self.identity)?
                .as_ref()
                .map(|c| &c.snapshot.digest)
                != self.cache.as_ref().map(|c| &c.snapshot.digest)
        {
            return Err(AgentError::Invalid(
                "DNS identity or cache changed during sync",
            ));
        }
        if changed {
            let cache = Cache {
                format_version: 1,
                controller: self.identity.controller.clone(),
                node_id: snapshot.payload.node_id.clone(),
                credential_hash: local::digest(&self.identity.node_credential),
                snapshot,
            };
            let bytes = serde_json::to_vec(&cache)
                .map_err(|_| AgentError::Invalid("cannot encode DNS cache"))?;
            let result = local::atomic_write(&path(&self.state), &bytes, self.cache.is_some());
            if result.is_ok() || result.as_ref().is_err_and(|error| error.committed()) {
                self.cache = Some(cache);
            }
            result?;
        }
        local::ensure_durable(&path(&self.state))?;
        let snapshot = &self.cache.as_ref().expect("persisted DNS").snapshot;
        if !snapshot.payload.dns.enabled {
            self.guard.disable();
            self.event("disabled", Some(snapshot));
        } else if self
            .guard
            .publish(zone, fresh_at + self.lease, snapshot.payload.revision)
        {
            self.event("active", Some(snapshot));
        } else {
            self.event("withdrawn", Some(snapshot));
            return Err(AgentError::Http {
                status: 409,
                retryable: true,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
