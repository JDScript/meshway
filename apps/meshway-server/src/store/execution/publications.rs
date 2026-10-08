use super::{
    authentication::authorize_node,
    bytes,
    cache::{self, HistoricalTarget},
    current_configuration, id, identity_digest,
    instances::{current_instance, grant_lease},
    json, now, subjects, user_binding, uuid,
};
use crate::configuration::ConfigurationBindings;
use crate::effect::Effect;
use crate::error::Error;
use crate::execution::NodePrincipal;
use crate::secrets::Sealer;
use meshway_configuration::NetworkConfiguration;
use meshway_configuration::execution::*;
use meshway_model::{
    BoundNodeKey, ConfigurationRef, Digest, InstanceId, KeyAlgorithm, NetworkId, NodeId,
    NodePublicKey, PublicationId, Revision, Version,
};
use meshway_policy::compile::{self, CompilationInput};
use meshway_protocol::execution::*;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeMap;

const MAX_DEMAND_TTL_SECONDS: u16 = 300;

pub(crate) fn select_profile(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: RoutingSelectionRequest,
) -> Result<(RoutingSelectionResponse, Vec<Effect>), Error> {
    let now = now()?;
    let p = authorize_node(tx, sealer, boot, token, now)?;
    current_instance(tx, boot, &p, &request.instance, now)?;
    let (_, cfg, bindings) = current_configuration(tx, sealer, p.network)?;
    if !compile::profile_allowed(
        &cfg,
        &subjects(tx, p.network, &bindings)?,
        p.node,
        &request.profile,
    )? {
        return Err(Error::PermissionDenied);
    }
    let digest = identity_digest(&cfg, &bindings, p.node)?;
    let current: Option<i64> = tx
        .query_row(
            "SELECT version FROM routing_selections WHERE network_id=?1 AND node_id=?2",
            params![
                p.network.as_uuid().as_bytes().as_slice(),
                p.node.as_uuid().as_bytes().as_slice()
            ],
            |r| r.get(0),
        )
        .optional()?;
    if current != request.expected_version.map(|v| v.get() as i64) {
        return Err(Error::BindingConflict);
    }
    let version =
        Version::new(current.unwrap_or(0) as u64 + 1).map_err(|_| Error::BindingConflict)?;
    tx.execute("INSERT INTO routing_selections(network_id,node_id,identity_digest,profile,version,selected_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(network_id,node_id) DO UPDATE SET identity_digest=excluded.identity_digest,profile=excluded.profile,version=excluded.version,selected_at=excluded.selected_at",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),digest.as_bytes().as_slice(),request.profile,version.get() as i64,now])?;
    tx.execute(
        "UPDATE execution_demands SET cancelled_at=?3 WHERE network_id=?1 AND node_id=?2 AND cancelled_at IS NULL",
        params![
            p.network.as_uuid().as_bytes().as_slice(),
            p.node.as_uuid().as_bytes().as_slice(), now
        ],
    )?;
    let publication = publish(tx, sealer, p.network, "refresh", now)?.publication;
    Ok((
        RoutingSelectionResponse {
            version,
            profile: request.profile,
            publication,
        },
        vec![Effect::Progress {
            network_id: p.network,
        }],
    ))
}

/// Who the configuration's subjects are, which nodes hold trusted keys, and
/// which profile each node selected under its current identity: the inputs
/// compilation and planning share for one configuration and its bindings.
pub(crate) struct CompilationContext {
    pub(crate) subjects: meshway_policy::SubjectContext,
    pub(crate) inventory: BTreeMap<NodeId, TrustedNode>,
    pub(crate) selected_profiles: BTreeMap<NodeId, String>,
}
pub(crate) fn compilation_context(
    tx: &Transaction<'_>,
    network: NetworkId,
    cfg: &NetworkConfiguration,
    bindings: &ConfigurationBindings,
) -> Result<CompilationContext, Error> {
    let mut selected_profiles = BTreeMap::new();
    let mut statement = tx.prepare(
        "SELECT node_id,identity_digest,profile FROM routing_selections WHERE network_id=?1",
    )?;
    let mut rows = statement.query([network.as_uuid().as_bytes().as_slice()])?;
    while let Some(row) = rows.next()? {
        let node: NodeId = id(row.get(0)?)?;
        let stored: Vec<u8> = row.get(1)?;
        if cfg.nodes.contains_key(&node) {
            let digest = identity_digest(cfg, bindings, node)?;
            if stored == digest.as_bytes() {
                selected_profiles.insert(node, row.get::<_, String>(2)?);
            }
        }
    }
    drop(rows);
    drop(statement);
    let subjects = subjects(tx, network, bindings)?;
    let inventory = trusted_inventory(tx, network, cfg, &subjects)?;
    Ok(CompilationContext {
        subjects,
        inventory,
        selected_profiles,
    })
}
fn trusted_inventory(
    tx: &Transaction<'_>,
    network: NetworkId,
    cfg: &NetworkConfiguration,
    subjects: &meshway_policy::SubjectContext,
) -> Result<BTreeMap<NodeId, TrustedNode>, Error> {
    let mut out = BTreeMap::new();
    let mut stmt=tx.prepare("SELECT n.id,k.id,k.public_key,k.generation,d.id,d.public_key,d.generation,e.capabilities FROM nodes n JOIN node_keys k ON k.network_id=n.network_id AND k.node_id=n.id AND k.purpose='control' AND k.retired_at IS NULL AND k.revoked_at IS NULL JOIN node_keys d ON d.network_id=n.network_id AND d.node_id=n.id AND d.purpose='dataplane' AND d.retired_at IS NULL AND d.revoked_at IS NULL LEFT JOIN node_execution e ON e.network_id=n.network_id AND e.node_id=n.id WHERE n.network_id=?1 AND n.credentials_revoked_at IS NULL")?;
    let mut rows = stmt.query([network.as_uuid().as_bytes().as_slice()])?;
    while let Some(r) = rows.next()? {
        let node = id(r.get(0)?)?;
        if !subjects.node_is_active(cfg, node) {
            continue;
        }
        let key = |offset| -> Result<BoundNodeKey, Error> {
            Ok(BoundNodeKey {
                id: id(r.get(offset)?)?,
                generation: Version::new(
                    r.get::<_, i64>(offset + 2)?
                        .try_into()
                        .map_err(|_| Error::decode("node_keys.generation"))?,
                )
                .map_err(|_| Error::decode("node_keys.generation"))?,
                public_key: NodePublicKey {
                    algorithm: if offset == 1 {
                        KeyAlgorithm::Ed25519
                    } else {
                        KeyAlgorithm::X25519
                    },
                    bytes: Digest::from_bytes(
                        r.get::<_, Vec<u8>>(offset + 1)?
                            .try_into()
                            .map_err(|_| Error::decode("node_keys.public_key"))?,
                    ),
                },
            })
        };
        let capabilities = r
            .get::<_, Option<Vec<u8>>>(7)?
            .map(|v| json(&v))
            .transpose()?
            .unwrap_or_default();
        out.insert(
            node,
            TrustedNode {
                control_key: key(1)?,
                dataplane_key: key(4)?,
                capabilities,
            },
        );
    }
    Ok(out)
}

pub(crate) fn check_diagnostic_compilation(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    expected: &BTreeMap<NodeId, TrustedNode>,
) -> Result<(), Error> {
    let (_, cfg, bindings) = current_configuration(tx, sealer, network)?;
    let current = trusted_inventory(tx, network, &cfg, &subjects(tx, network, &bindings)?)?;
    if expected
        .iter()
        .any(|(node, trusted)| current.get(node) != Some(trusted))
    {
        return Err(Error::StateConflict);
    }
    Ok(())
}
/// Transaction-local immutable publication context. A shared old publication
/// is decrypted once, even when thousands of nodes still target its slices.
fn active_publications(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
) -> Result<BTreeMap<PublicationId, CompiledPublication>, Error> {
    let mut statement = tx.prepare(
        "SELECT id,payload FROM execution_publications x WHERE network_id=?1 AND withdrawn_at IS NULL
         AND EXISTS(SELECT 1 FROM node_execution e WHERE e.network_id=x.network_id AND e.target_publication_id=x.id)",
    )?;
    let mut rows = statement.query([network.as_uuid().as_bytes().as_slice()])?;
    let mut publications = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let publication = id(row.get(0)?)?;
        let payload: Vec<u8> = row.get(1)?;
        match cache::read(tx, sealer, network, publication, &payload)? {
            cache::Stored::Current(compiled) => {
                publications.insert(publication, compiled);
            }
            cache::Stored::Obsolete { .. } => cache::retire(tx, network, publication, now()?)?,
        }
    }
    Ok(publications)
}

pub(crate) fn publish(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    cause: &str,
    now: i64,
) -> Result<CompiledPublication, Error> {
    let (revision, _, bindings) = current_configuration(tx, sealer, network)?;
    let rule_sets =
        crate::resources::bindings::at_revision(tx, network, revision, &bindings.rule_sets)?;
    publish_with_rule_sets(tx, sealer, network, cause, now, &rule_sets)
}

pub(crate) fn publish_with_rule_sets(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    cause: &str,
    now: i64,
    rule_set_bindings: &crate::resources::bindings::Bindings,
) -> Result<CompiledPublication, Error> {
    let (revision, cfg, bindings) = current_configuration(tx, sealer, network)?;
    let rule_sets =
        crate::resources::bindings::compilation_contents(tx, network, rule_set_bindings)?;
    let CompilationContext {
        subjects,
        inventory,
        selected_profiles,
    } = compilation_context(tx, network, &cfg, &bindings)?;
    let mut stmt = tx.prepare(
        "SELECT payload FROM execution_demands WHERE network_id=?1 AND expires_at>?2 AND cancelled_at IS NULL ORDER BY id",
    )?;
    let demands = stmt
        .query_map(params![network.as_uuid().as_bytes().as_slice(), now], |r| {
            r.get::<_, Vec<u8>>(0)
        })?
        .map(|v| {
            let d: Demand = json(&v?)?;
            Ok((d.id, d))
        })
        .collect::<Result<BTreeMap<_, _>, Error>>()?;
    drop(stmt);
    let publication =
        PublicationId::new(uuid()?).map_err(|_| Error::Internal("generated publication ID"))?;
    let mut diagnostics = crate::execution::diagnostics::collect(tx, network, now)?;
    let compile_diagnostics = |diagnostics: &BTreeMap<_, _>| {
        compile::compile_with_diagnostics(
            CompilationInput {
                network,
                revision,
                publication,
                configuration: &cfg,
                subjects: &subjects,
                inventory: &inventory,
                demands: &demands,
                dns: &[],
                rule_sets: &rule_sets,
                selected_profiles: &selected_profiles,
                now_ms: now as u64,
            },
            diagnostics,
        )
    };
    let mut compiled = compile_diagnostics(&diagnostics)?;
    // The first compiler pass is the sole Access/routing/topology authority.
    // Keep its denial distinct from a permitted path whose runtime is absent.
    // These private decisions are committed with the final safe publication;
    // the unsupported first-pass permits are never published to any Node.
    use crate::execution::diagnostics::{
        DiagnosticCompilation, DiagnosticDecision, supports_diagnostics,
    };
    let mut diagnostic_decisions = BTreeMap::new();
    let mut unsupported = std::collections::BTreeSet::new();
    let mut recompile = false;
    for (id, demand) in &mut diagnostics {
        let permit = compiled
            .nodes
            .get(&demand.source)
            .and_then(|node| node.configuration.diagnostics.get(id));
        let decision = match permit {
            None => DiagnosticDecision::Denied,
            Some(permit) => {
                let supported: Vec<_> = permit
                    .paths
                    .iter()
                    .filter(|path| {
                        path.nodes.iter().all(|node| {
                            compiled.nodes.get(node).is_some_and(|node| {
                                supports_diagnostics(&node.configuration.self_keys.capabilities)
                            })
                        })
                    })
                    .collect();
                if supported.is_empty() {
                    unsupported.insert(*id);
                    recompile = true;
                    DiagnosticDecision::Unsupported {
                        paths: permit.paths.iter().map(|path| path.nodes.clone()).collect(),
                    }
                } else {
                    if supported.len() != permit.paths.len() {
                        // Auto may use a fully capable authorized candidate. Pin
                        // this compilation to one such complete path, then ask
                        // the same compiler to validate it again. Never splice
                        // paths or bypass a required waypoint.
                        demand.path = Some(supported[0].nodes.clone());
                        recompile = true;
                    }
                    DiagnosticDecision::Executable
                }
            }
        };
        diagnostic_decisions.insert(*id, (demand.source, decision));
    }
    if recompile {
        diagnostics.retain(|id, _| !unsupported.contains(id));
        compiled = compile_diagnostics(&diagnostics)?;
    }
    let mut stmt = tx.prepare(
        "SELECT node_id,target_publication_id,execution_expires_at FROM node_execution
         WHERE network_id=?1 AND target_publication_id IS NOT NULL",
    )?;
    let previous = stmt
        .query_map([network.as_uuid().as_bytes().as_slice()], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, Option<i64>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    let mut supersedes = std::collections::BTreeSet::new();
    let active = active_publications(tx, sealer, network)?;
    // Compare the actual node slice, not the network-wide input digest. A new
    // unrelated demand must not invalidate an already-installed publication.
    for (node, old_publication, expiry) in previous {
        let node: NodeId = id(node)?;
        let old_publication: PublicationId = id(old_publication)?;
        let old = active
            .get(&old_publication)
            .and_then(|publication| publication.nodes.get(&node));
        let unchanged = if let (Some(old), Some(new)) = (old, compiled.nodes.get(&node)) {
            let mut comparable = new.configuration.clone();
            comparable.publication = old.configuration.publication;
            comparable.dependency_digest = old.configuration.dependency_digest;
            comparable == old.configuration && expiry.is_none_or(|expiry| expiry > now)
        } else {
            false
        };
        if unchanged {
            compiled.nodes.remove(&node);
        } else {
            supersedes.insert(old_publication);
            tx.execute(
                "UPDATE node_execution SET target_publication_id=NULL,execution_expires_at=NULL
                 WHERE network_id=?1 AND node_id=?2",
                params![
                    network.as_uuid().as_bytes().as_slice(),
                    node.as_uuid().as_bytes().as_slice()
                ],
            )?;
        }
    }
    tx.execute(
        "UPDATE networks SET publication_sequence=publication_sequence+1 WHERE id=?1",
        [network.as_uuid().as_bytes().as_slice()],
    )?;
    let sequence: i64 = tx.query_row(
        "SELECT publication_sequence FROM networks WHERE id=?1",
        [network.as_uuid().as_bytes().as_slice()],
        |r| r.get(0),
    )?;
    let payload = zeroize::Zeroizing::new(bytes(&compiled)?);
    let sealed = sealer.seal_execution(network, publication, &payload)?;
    tx.execute(
        "INSERT INTO execution_publications(id,network_id,sequence,revision,cause,supersedes,dependency_digest,payload,created_at,rule_set_bindings)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            publication.as_uuid().as_bytes().as_slice(),
            network.as_uuid().as_bytes().as_slice(),
            sequence,
            revision.get() as i64,
            cause,
            bytes(&supersedes)?,
            compiled.dependency_digest.as_bytes().as_slice(),
            sealed.as_bytes(),
            now,
            bytes(rule_set_bindings)?,
        ],
    )?;
    // All demands were compiled together; only changed slices become targets.
    for node in compiled.nodes.keys() {
        tx.execute(
            "UPDATE node_execution SET target_publication_id=?1 WHERE network_id=?2 AND node_id=?3",
            params![
                publication.as_uuid().as_bytes().as_slice(),
                network.as_uuid().as_bytes().as_slice(),
                node.as_uuid().as_bytes().as_slice()
            ],
        )?;
    }
    for old in supersedes {
        tx.execute(
            "UPDATE execution_publications SET withdrawn_at=COALESCE(withdrawn_at,?1)
             WHERE network_id=?2 AND id=?3 AND NOT EXISTS(
                 SELECT 1 FROM node_execution WHERE network_id=?2 AND target_publication_id=?3)",
            params![
                now,
                network.as_uuid().as_bytes().as_slice(),
                old.as_uuid().as_bytes().as_slice()
            ],
        )?;
    }
    for (id, (source, decision)) in diagnostic_decisions {
        let (_, target) =
            target_for_node(tx, sealer, network, source)?.ok_or(Error::StateConflict)?;
        let configuration = ConfigurationRef {
            revision: target.configuration.revision,
            execution_publication: target.configuration.publication,
            digest: target.digest,
        };
        let participant_ids: std::collections::BTreeSet<_> = match &decision {
            DiagnosticDecision::Unsupported { paths } => paths.iter().flatten().copied().collect(),
            _ => target
                .configuration
                .peers
                .keys()
                .copied()
                .chain(std::iter::once(source))
                .collect(),
        };
        let participants = participant_ids
            .into_iter()
            .map(|node| {
                inventory
                    .get(&node)
                    .cloned()
                    .map(|trusted| (node, trusted))
                    .ok_or(Error::StateConflict)
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        super::diagnostics::record_compilation(
            tx,
            id,
            &DiagnosticCompilation {
                configuration,
                participants,
                decision,
            },
        )?;
    }
    Ok(compiled)
}
pub(crate) fn target(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    p: &NodePrincipal,
) -> Result<Option<(PublicationId, CompiledNode)>, Error> {
    target_for_node(tx, sealer, p.network, p.node)
}
/// SQL facts only. The caller must authorize its viewer in this transaction.
pub(crate) fn target_for_node(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    node: NodeId,
) -> Result<Option<(PublicationId, CompiledNode)>, Error> {
    let row:Option<(Vec<u8>,Vec<u8>)>=tx.query_row("SELECT x.id,x.payload FROM node_execution e JOIN execution_publications x ON x.network_id=e.network_id AND x.id=e.target_publication_id WHERE e.network_id=?1 AND e.node_id=?2 AND x.withdrawn_at IS NULL",params![network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((publication, payload)) = row else {
        return Ok(None);
    };
    let publication = id(publication)?;
    let Some(mut compiled) = cache::read(tx, sealer, network, publication, &payload)?.current()
    else {
        cache::retire(tx, network, publication, now()?)?;
        return Ok(None);
    };
    let node = compiled
        .nodes
        .remove(&node)
        .ok_or(Error::decode("execution_publications.payload"))?;
    Ok(Some((publication, node)))
}
pub(super) fn historical_target(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    p: &NodePrincipal,
    reference: &ConfigurationRef,
) -> Result<Option<CompiledNode>, Error> {
    Ok(
        historical_target_for_node(tx, sealer, p.network, p.node, reference)?
            .and_then(|target| target.current),
    )
}

pub(super) fn historical_reference(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    p: &NodePrincipal,
    reference: &ConfigurationRef,
) -> Result<bool, Error> {
    Ok(historical_target_for_node(tx, sealer, p.network, p.node, reference)?.is_some())
}

fn historical_target_for_node(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    node: NodeId,
    reference: &ConfigurationRef,
) -> Result<Option<HistoricalTarget>, Error> {
    let payload:Option<Vec<u8>>=tx.query_row("SELECT payload FROM execution_publications WHERE network_id=?1 AND id=?2 AND revision=?3",params![network.as_uuid().as_bytes().as_slice(),reference.execution_publication.as_uuid().as_bytes().as_slice(),reference.revision.get() as i64],|r|r.get(0)).optional()?;
    let Some(payload) = payload else {
        return Ok(None);
    };
    Ok(cache::read(
        tx,
        sealer,
        network,
        reference.execution_publication,
        &payload,
    )?
    .historical(node)
    .filter(|target| &target.reference == reference))
}

/// Only accepted evidence for the current fenced instance enters historical
/// State contexts. This lookup never confers installation/execution authority.
pub(crate) fn receipt_history_targets(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    node: NodeId,
    instance: InstanceId,
    now: i64,
) -> Result<Vec<HistoricalTarget>, Error> {
    let mut statement = tx.prepare(
        "SELECT r.publication_id,x.revision,r.digest FROM execution_receipts r
         JOIN execution_publications x ON x.network_id=r.network_id AND x.id=r.publication_id
         WHERE r.network_id=?1 AND r.node_id=?2 AND r.instance_id=?3 AND r.received_at>=?4
         GROUP BY r.publication_id,x.revision,r.digest ORDER BY max(r.received_at) DESC LIMIT 32",
    )?;
    let rows = statement
        .query_map(
            params![
                network.as_uuid().as_bytes().as_slice(),
                node.as_uuid().as_bytes().as_slice(),
                instance.as_uuid().as_bytes().as_slice(),
                now.saturating_sub(300_000)
            ],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let mut targets = Vec::new();
    for (publication, revision, digest) in rows {
        let reference = ConfigurationRef {
            revision: Revision::new(
                revision
                    .try_into()
                    .map_err(|_| Error::decode("execution_publications.revision"))?,
            )
            .map_err(|_| Error::decode("execution_publications.revision"))?,
            execution_publication: id(publication)?,
            digest: Digest::from_bytes(
                digest
                    .try_into()
                    .map_err(|_| Error::decode("execution_receipts.digest"))?,
            ),
        };
        if let Some(target) = historical_target_for_node(tx, sealer, network, node, &reference)? {
            targets.push(target);
        }
    }
    Ok(targets)
}
/// Targets this node held in the network's recent publications. A node that
/// saved a publication just before the next one was issued reports that saved
/// reference truthfully; it is admitted as history (never as installation
/// authority) so successive revisions do not turn honest reports into
/// contract errors.
pub(crate) fn recent_targets_for_node(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    node: NodeId,
    now: i64,
) -> Result<Vec<HistoricalTarget>, Error> {
    let mut statement = tx.prepare(
        "SELECT id,payload FROM execution_publications WHERE network_id=?1 AND created_at>=?2
         ORDER BY sequence DESC LIMIT 8",
    )?;
    let rows = statement
        .query_map(
            params![
                network.as_uuid().as_bytes().as_slice(),
                now.saturating_sub(RECENT_PUBLICATION_WINDOW_MS)
            ],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let mut targets = Vec::new();
    for (publication, payload) in rows {
        let publication: PublicationId = id(publication)?;
        if let Some(target) =
            cache::read(tx, sealer, network, publication, &payload)?.historical(node)
        {
            targets.push(target);
        }
    }
    Ok(targets)
}
/// How long a replaced publication stays an admissible report reference.
const RECENT_PUBLICATION_WINDOW_MS: i64 = 15 * 60 * 1000;
pub(crate) fn check_publication(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    node: NodeId,
    publication: PublicationId,
    now: i64,
) -> Result<(), Error> {
    let payload:Vec<u8>=tx.query_row("SELECT x.payload FROM execution_publications x JOIN networks n ON n.id=x.network_id AND n.current_revision=x.revision WHERE x.network_id=?1 AND x.id=?2 AND x.withdrawn_at IS NULL",params![network.as_uuid().as_bytes().as_slice(),publication.as_uuid().as_bytes().as_slice()],|r|r.get(0)).optional()?.ok_or(Error::BindingConflict)?;
    let compiled = cache::read(tx, sealer, network, publication, &payload)?
        .current()
        .ok_or(Error::BindingConflict)?;
    let (_, cfg, bindings) = current_configuration(tx, sealer, network)?;
    let current = trusted_inventory(tx, network, &cfg, &subjects(tx, network, &bindings)?)?;
    let node = compiled.nodes.get(&node).ok_or(Error::BindingConflict)?;
    if current.get(&node.configuration.node_id) != Some(&node.configuration.self_keys) {
        return Err(Error::BindingConflict);
    }
    for (id, peer) in &node.configuration.peers {
        if current.get(id).is_none_or(|keys| {
            keys.control_key != peer.control_key || keys.dataplane_key != peer.dataplane_key
        }) {
            return Err(Error::BindingConflict);
        }
    }
    if node
        .configuration
        .permits
        .values()
        .any(|p| p.expires_at_ms <= now as u64)
    {
        return Err(Error::BindingConflict);
    }
    Ok(())
}
pub(super) fn refresh_expired_demands(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    now: i64,
) -> Result<bool, Error> {
    if retire_expired_demands(tx, network, now)? {
        publish(tx, sealer, network, "refresh", now)?;
        return Ok(true);
    }
    Ok(false)
}
fn retire_expired_demands(
    tx: &Transaction<'_>,
    network: NetworkId,
    now: i64,
) -> Result<bool, Error> {
    Ok(tx.execute(
        "UPDATE execution_demands SET cancelled_at=?2 WHERE network_id=?1 AND expires_at<=?2 AND cancelled_at IS NULL",
        params![network.as_uuid().as_bytes().as_slice(), now],
    )? > 0)
}
pub(crate) fn revoke_execution(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    node: NodeId,
    now: i64,
) -> Result<(), Error> {
    for (publication, compiled) in active_publications(tx, sealer, network)? {
        for (target_node, target) in compiled.nodes {
            if target_node != node && !target.configuration.peers.contains_key(&node) {
                continue;
            }
            tx.execute(
                "UPDATE node_execution SET target_publication_id=NULL,execution_expires_at=NULL
                 WHERE network_id=?1 AND node_id=?2 AND target_publication_id=?3",
                params![
                    network.as_uuid().as_bytes().as_slice(),
                    target_node.as_uuid().as_bytes().as_slice(),
                    publication.as_uuid().as_bytes().as_slice()
                ],
            )?;
        }
        tx.execute(
            "UPDATE execution_publications SET withdrawn_at=COALESCE(withdrawn_at,?1)
             WHERE network_id=?2 AND id=?3 AND NOT EXISTS(
                 SELECT 1 FROM node_execution WHERE network_id=?2 AND target_publication_id=?3)",
            params![
                now,
                network.as_uuid().as_bytes().as_slice(),
                publication.as_uuid().as_bytes().as_slice()
            ],
        )?;
    }
    tx.execute(
        "UPDATE execution_demands SET cancelled_at=?3 WHERE network_id=?1 AND node_id=?2 AND cancelled_at IS NULL",
        params![
            network.as_uuid().as_bytes().as_slice(),
            node.as_uuid().as_bytes().as_slice(), now
        ],
    )?;
    Ok(())
}
/// Ends every live control session, open demand and instance of one Node. The
/// Node must authenticate again and acquire a new instance; its keys, target
/// and publication references are untouched.
pub(crate) fn retire_node_sessions(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    now: i64,
) -> Result<(), Error> {
    tx.execute(
        "DELETE FROM node_sessions WHERE network_id=?1 AND node_id=?2",
        params![
            network.as_uuid().as_bytes().as_slice(),
            node.as_uuid().as_bytes().as_slice()
        ],
    )?;
    tx.execute(
        "UPDATE execution_demands SET cancelled_at=?3 WHERE network_id=?1 AND node_id=?2 AND cancelled_at IS NULL",
        params![
            network.as_uuid().as_bytes().as_slice(),
            node.as_uuid().as_bytes().as_slice(),
            now
        ],
    )?;
    tx.execute(
        "UPDATE node_execution SET instance_expires_at=?1,execution_expires_at=NULL WHERE network_id=?2 AND node_id=?3",
        params![
            now,
            network.as_uuid().as_bytes().as_slice(),
            node.as_uuid().as_bytes().as_slice()
        ],
    )?;
    Ok(())
}
/// Config owner invokes within its source commit transaction after writing the
/// authoritative target. Removed nodes are permanent revocations, including JSONC deletion.
pub(crate) fn apply_configuration_effects(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    previous: &NetworkConfiguration,
    previous_bindings: &ConfigurationBindings,
    target: &NetworkConfiguration,
    now: i64,
) -> Result<CompiledPublication, Error> {
    let (revision, committed, bindings) = current_configuration(tx, sealer, network)?;
    if &committed != target {
        return Err(Error::Internal(
            "committed configuration differs from the prepared target",
        ));
    }
    for (node, old) in &previous.nodes {
        if target
            .nodes
            .get(node)
            .is_some_and(|new| match (&old.identity, &new.identity) {
                (
                    meshway_configuration::NodeIdentity::User { .. },
                    meshway_configuration::NodeIdentity::User { .. },
                ) => {
                    user_binding(previous, previous_bindings, *node)
                        != user_binding(target, &bindings, *node)
                }
                _ => new.identity != old.identity,
            })
        {
            retire_node_sessions(tx, network, *node, now)?;
        }
    }
    for (node, old) in &previous.nodes {
        let addresses = [
            old.addresses.ipv4.map(std::net::IpAddr::V4),
            old.addresses.ipv6.map(std::net::IpAddr::V6),
        ];
        let new = target
            .nodes
            .get(node)
            .map(|n| {
                [
                    n.addresses.ipv4.map(std::net::IpAddr::V4),
                    n.addresses.ipv6.map(std::net::IpAddr::V6),
                ]
            })
            .unwrap_or([None, None]);
        for address in addresses
            .into_iter()
            .flatten()
            .filter(|a| !new.contains(&Some(*a)))
        {
            tx.execute("INSERT INTO address_retirements(network_id,address,previous_node_id,removed_revision,release_after) VALUES(?1,?2,?3,?4,NULL)",params![network.as_uuid().as_bytes().as_slice(),address.to_string(),node.as_uuid().as_bytes().as_slice(),revision.get() as i64])?;
        }
    }
    for node in previous
        .nodes
        .keys()
        .filter(|id| !target.nodes.contains_key(id))
    {
        tx.execute("UPDATE nodes SET credentials_revoked_at=COALESCE(credentials_revoked_at,?1) WHERE network_id=?2 AND id=?3",params![now,network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice()])?;
        revoke_execution(tx, sealer, network, *node, now)?;
    }
    publish(tx, sealer, network, "configuration", now)
}

/// Fixed apply participants include offline nodes present in the new slices;
/// later demand/refresh publications must never change this version's targets.
pub(crate) fn configuration_targets(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    revision: Revision,
    publications: &[PublicationId],
) -> Result<BTreeMap<NodeId, ConfigurationRef>, Error> {
    let mut targets = BTreeMap::new();
    for publication in publications {
        let payload: Vec<u8> = tx.query_row(
            "SELECT payload FROM execution_publications WHERE network_id=?1 AND id=?2 AND revision=?3 AND cause='configuration' AND withdrawn_at IS NULL",
            params![network.as_uuid().as_bytes().as_slice(), publication.as_uuid().as_bytes().as_slice(), revision.get() as i64],
            |row| row.get(0),
        )?;
        let compiled = cache::read(tx, sealer, network, *publication, &payload)?
            .current()
            .ok_or(Error::BindingConflict)?;
        for (node, slice) in compiled.nodes {
            if slice.configuration.network_id != network || slice.configuration.node_id != node
                || slice.configuration.revision != revision || slice.configuration.publication != *publication
                || !tx.query_row("SELECT EXISTS(SELECT 1 FROM nodes WHERE network_id=?1 AND id=?2 AND credentials_revoked_at IS NULL)", params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice()], |row| row.get::<_,bool>(0))?
                || targets.insert(node, ConfigurationRef { revision, execution_publication: *publication, digest: slice.digest }).is_some()
            {
                return Err(Error::BindingConflict);
            }
        }
    }
    Ok(targets)
}
pub(crate) fn demand(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: DemandRequest,
) -> Result<(DemandResponse, Vec<Effect>), Error> {
    if !(1..=MAX_DEMAND_TTL_SECONDS).contains(&request.duration_seconds) {
        return Err(Error::InvalidRequest);
    }
    let now = now()?;
    let p = authorize_node(tx, sealer, boot, token, now)?;
    current_instance(tx, boot, &p, &request.instance, now)?;
    let digest = crate::execution::proof::request_digest(&request)?;
    let previous: Option<(Vec<u8>,i64,Option<i64>)> = tx.query_row(
        "SELECT request_digest,expires_at,cancelled_at FROM execution_demands WHERE network_id=?1 AND node_id=?2 AND id=?3",
        params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),request.request_id.as_uuid().as_bytes().as_slice()],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional()?;
    if previous.as_ref().is_some_and(|(v, expires, cancelled)| {
        v != digest.as_bytes() || *expires <= now || cancelled.is_some()
    }) {
        return Err(Error::BindingConflict);
    }
    if previous.is_some() {
        // Recovering a still-valid demand must not depend on whether some
        // other, expired demand in the same slice was already refreshed by a
        // renew or configuration read: retire it here first.
        let refreshed = refresh_expired_demands(tx, sealer, p.network, now)?;
        let (publication, target) = target(tx, sealer, &p)?.ok_or(Error::BindingConflict)?;
        check_publication(tx, sealer, p.network, p.node, publication, now)?;
        let decision = target
            .configuration
            .decisions
            .get(&request.request_id)
            .cloned()
            .ok_or(Error::BindingConflict)?;
        let effects = if refreshed {
            vec![Effect::Progress {
                network_id: p.network,
            }]
        } else {
            Vec::new()
        };
        return Ok((
            DemandResponse {
                publication,
                decision,
            },
            effects,
        ));
    }
    if previous.is_none() {
        let d = Demand {
            id: request.request_id,
            source: p.node,
            target: request.target,
            protocol: request.protocol,
            port: request.port,
            profile: request.profile,
            expires_at_ms: (now + i64::from(request.duration_seconds) * 1000) as u64,
        };
        tx.execute(
            "INSERT INTO execution_demands(network_id,node_id,id,instance_id,request_digest,payload,expires_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),
                d.id.as_uuid().as_bytes().as_slice(),request.instance.instance_id.as_uuid().as_bytes().as_slice(),
                digest.as_bytes().as_slice(),bytes(&d)?,d.expires_at_ms as i64],
        )?;
    }
    let compiled = publish(tx, sealer, p.network, "demand", now)?;
    let decision = compiled
        .nodes
        .get(&p.node)
        .and_then(|n| n.configuration.decisions.get(&request.request_id))
        .cloned()
        .ok_or(Error::BindingConflict)?;
    grant_lease(tx, sealer, &p, now)?;
    Ok((
        DemandResponse {
            publication: compiled.publication,
            decision,
        },
        vec![Effect::Progress {
            network_id: p.network,
        }],
    ))
}
pub(crate) fn renew_demand(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: RenewDemandRequest,
) -> Result<(DemandResponse, Vec<Effect>), Error> {
    let now = now()?;
    let p = authorize_node(tx, sealer, boot, token, now)?;
    current_instance(tx, boot, &p, &request.instance, now)?;
    // Scope and liveness are checked before considering an idempotent replay.
    // Neither a late retry nor a replacement process can resurrect this intent.
    let previous: Option<(Vec<u8>, i64)> = tx
        .query_row(
            "SELECT payload,expires_at FROM execution_demands
         WHERE network_id=?1 AND node_id=?2 AND id=?3 AND instance_id=?4
         AND expires_at>?5 AND cancelled_at IS NULL",
            params![
                p.network.as_uuid().as_bytes().as_slice(),
                p.node.as_uuid().as_bytes().as_slice(),
                request.request_id.as_uuid().as_bytes().as_slice(),
                request.instance.instance_id.as_uuid().as_bytes().as_slice(),
                now
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (payload, expires) = previous.ok_or(Error::BindingConflict)?;
    let mut demand: Demand = json(&payload)?;
    if demand.id != request.request_id
        || demand.source != p.node
        || demand.expires_at_ms != expires as u64
    {
        return Err(Error::decode("execution_demands.payload"));
    }
    let extended = request.expires_at_ms > demand.expires_at_ms;
    let changed = if extended {
        let requested = i64::try_from(request.expires_at_ms).map_err(|_| Error::InvalidRequest)?;
        let instance_expires: i64 = tx.query_row(
            "SELECT instance_expires_at FROM node_execution WHERE network_id=?1 AND node_id=?2",
            params![
                p.network.as_uuid().as_bytes().as_slice(),
                p.node.as_uuid().as_bytes().as_slice()
            ],
            |row| row.get(0),
        )?;
        if requested <= now
            || requested > instance_expires
            || requested > now + i64::from(MAX_DEMAND_TTL_SECONDS) * 1000
        {
            return Err(Error::InvalidRequest);
        }
        demand.expires_at_ms = request.expires_at_ms;
        // Keep the original create digest: retrying create never extends expiry.
        tx.execute(
            "UPDATE execution_demands SET payload=?4,expires_at=?5 WHERE network_id=?1 AND node_id=?2 AND id=?3",
            params![p.network.as_uuid().as_bytes().as_slice(), p.node.as_uuid().as_bytes().as_slice(),
                request.request_id.as_uuid().as_bytes().as_slice(), bytes(&demand)?, requested],
        )?;
        // Retire expired rows in the same transaction as the complete snapshot.
        retire_expired_demands(tx, p.network, now)?;
        publish(tx, sealer, p.network, "demand", now)?;
        grant_lease(tx, sealer, &p, now)?;
        true
    } else {
        // Cleanup may withdraw other expired permits, but replay itself neither
        // publishes nor grants an execution lease.
        refresh_expired_demands(tx, sealer, p.network, now)?
    };
    // An unavailable decision can be identical after renewal, so publication
    // deduplication may retain the source's previous immutable target.
    let (publication, target) = target(tx, sealer, &p)?.ok_or(Error::BindingConflict)?;
    check_publication(tx, sealer, p.network, p.node, publication, now)?;
    let decision = target
        .configuration
        .decisions
        .get(&request.request_id)
        .cloned()
        .ok_or(Error::BindingConflict)?;
    Ok((
        DemandResponse {
            publication,
            decision,
        },
        if changed {
            vec![Effect::Progress {
                network_id: p.network,
            }]
        } else {
            Vec::new()
        },
    ))
}

pub(crate) fn cancel_demand(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: CancelDemandRequest,
) -> Result<((), Vec<Effect>), Error> {
    let now = now()?;
    let p = authorize_node(tx, sealer, boot, token, now)?;
    current_instance(tx, boot, &p, &request.instance, now)?;
    let changed = tx.execute(
        "UPDATE execution_demands SET cancelled_at=?4 WHERE network_id=?1 AND node_id=?2 AND id=?3 AND cancelled_at IS NULL",
        params![
            p.network.as_uuid().as_bytes().as_slice(),
            p.node.as_uuid().as_bytes().as_slice(),
            request.request_id.as_uuid().as_bytes().as_slice(), now
        ],
    )? > 0;
    if changed {
        publish(tx, sealer, p.network, "demand", now)?;
    }
    let effects = if changed {
        vec![Effect::Progress {
            network_id: p.network,
        }]
    } else {
        Vec::new()
    };
    Ok(((), effects))
}
