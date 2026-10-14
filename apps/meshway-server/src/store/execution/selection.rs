//! Routing profile selection: the one commit the Node endpoint and the user
//! endpoint share, its activity record, and the read of a Node's selection.
//! The persistent selection is usage state, not configuration: it is kept per
//! Node under the identity it was made for, and `meshway-policy` decides what
//! it means under the current configuration.
use super::{
    authentication::authorize_node, current_configuration, id, identity_digest,
    instances::current_instance, now, subjects, user_binding,
};
use crate::configuration::ConfigurationBindings;
use crate::effect::Effect;
use crate::error::Error;
use crate::secrets::Sealer;
use meshway_configuration::NetworkConfiguration;
use meshway_configuration::selection::{RoutingSelectionState, SelectionUnavailable};
use meshway_model::{InstanceId, MembershipRole, NetworkId, NodeId, RequestId, UserId, Version};
use meshway_protocol::{RoutingProfileUnavailableParams, execution::*};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeMap;

/// Who commits a selection; recorded as the activity's actor.
#[derive(Clone, Copy)]
enum Actor {
    Node,
    User(UserId),
}

/// `routing_selections.profile` of a cleared selection (no profile name is
/// empty). The row stays so that versions remain monotonic; the matching
/// `routing.select` activity carries the same value as its target profile.
const CLEARED: &str = "";

/// Every configured Node's persistent selection under its current identity:
/// its profile and version. A cleared selection or one made under an earlier
/// identity is none. The one reading compilation, netmap versions and the
/// selection state share.
pub(crate) fn selections(
    tx: &Transaction<'_>,
    network: NetworkId,
    cfg: &NetworkConfiguration,
    bindings: &ConfigurationBindings,
) -> Result<BTreeMap<NodeId, (String, Version)>, Error> {
    let mut statement = tx.prepare(
        "SELECT node_id,identity_digest,profile,version FROM routing_selections WHERE network_id=?1",
    )?;
    let mut rows = statement.query([network.as_uuid().as_bytes().as_slice()])?;
    let mut out = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let node: NodeId = id(row.get(0)?)?;
        let digest: Vec<u8> = row.get(1)?;
        let profile: String = row.get(2)?;
        if profile == CLEARED
            || !cfg.nodes.contains_key(&node)
            || digest != identity_digest(cfg, bindings, node)?.as_bytes()
        {
            continue;
        }
        let version = u64::try_from(row.get::<_, i64>(3)?)
            .ok()
            .and_then(|version| Version::new(version).ok())
            .ok_or(Error::decode("routing_selections.version"))?;
        out.insert(node, (profile, version));
    }
    Ok(out)
}

/// The Node's persistent selection under its current identity.
fn stored(
    tx: &Transaction<'_>,
    network: NetworkId,
    cfg: &NetworkConfiguration,
    bindings: &ConfigurationBindings,
    node: NodeId,
) -> Result<Option<(String, Version)>, Error> {
    Ok(selections(tx, network, cfg, bindings)?.remove(&node))
}

/// A configured Node's selection as the current configuration decides it.
pub(crate) fn state(
    tx: &Transaction<'_>,
    network: NetworkId,
    cfg: &NetworkConfiguration,
    bindings: &ConfigurationBindings,
    subjects: &meshway_policy::SubjectContext,
    node: NodeId,
) -> Result<RoutingSelectionState, Error> {
    let selected = stored(tx, network, cfg, bindings, node)?;
    Ok(meshway_policy::compile::routing_selection(
        cfg,
        subjects,
        selected.as_ref().map(|(profile, _)| profile.as_str()),
        selected.as_ref().map(|(_, version)| *version),
        node,
    )?)
}

/// The one selection commit: `Some(profile)` selects it, `None` clears the
/// selection so that the assignment's default applies. `expected` is the
/// Node endpoint's compare-and-swap base; a user's change replaces whatever
/// is current. Clearing without a current selection changes nothing.
#[allow(clippy::too_many_arguments)]
fn commit(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    node: NodeId,
    actor: Actor,
    request: RequestId,
    profile: Option<String>,
    expected: Option<Option<Version>>,
    now: i64,
) -> Result<(RoutingSelectionState, Vec<Effect>), Error> {
    let (_, cfg, bindings) = current_configuration(tx, sealer, network)?;
    if !cfg.nodes.contains_key(&node) {
        return Err(Error::NotFound);
    }
    let revoked: bool = tx.query_row(
        "SELECT credentials_revoked_at IS NOT NULL FROM nodes WHERE network_id=?1 AND id=?2",
        params![
            network.as_uuid().as_bytes().as_slice(),
            node.as_uuid().as_bytes().as_slice()
        ],
        |r| r.get(0),
    )?;
    if revoked {
        return Err(Error::StateConflict);
    }
    let subjects = subjects(tx, network, &bindings)?;
    let current = state(tx, network, &cfg, &bindings, &subjects, node)?;
    if let Some(profile) = &profile
        && !meshway_policy::profile_allowed(&cfg, &subjects, node, profile)?
    {
        let allowed: Vec<String> = current.profiles.iter().map(|p| p.name.clone()).collect();
        return Err(Error::RoutingProfileUnavailable(
            RoutingProfileUnavailableParams {
                reason: if allowed.contains(profile) {
                    SelectionUnavailable::Locked
                } else {
                    SelectionUnavailable::NotAllowed
                },
                allowed,
            },
        ));
    }
    if expected.is_some_and(|expected| expected != current.version) {
        return Err(Error::VersionConflict);
    }
    if profile.is_none() && current.version.is_none() {
        return Ok((current, Vec::new()));
    }
    let profile = profile.unwrap_or_else(|| CLEARED.to_owned());
    // Versions stay monotonic across identities: a selection is never
    // reissued under a version an earlier selection used.
    let previous: Option<i64> = tx
        .query_row(
            "SELECT version FROM routing_selections WHERE network_id=?1 AND node_id=?2",
            params![
                network.as_uuid().as_bytes().as_slice(),
                node.as_uuid().as_bytes().as_slice()
            ],
            |r| r.get(0),
        )
        .optional()?;
    let version =
        Version::new(previous.unwrap_or(0) as u64 + 1).map_err(|_| Error::VersionConflict)?;
    let digest = identity_digest(&cfg, &bindings, node)?;
    tx.execute("INSERT INTO routing_selections(network_id,node_id,identity_digest,profile,version,selected_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(network_id,node_id) DO UPDATE SET identity_digest=excluded.identity_digest,profile=excluded.profile,version=excluded.version,selected_at=excluded.selected_at",params![network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice(),digest.as_bytes().as_slice(),profile,version.get() as i64,now])?;
    let (kind, user, actor_node) = match actor {
        Actor::Node => ("node", None, Some(node)),
        Actor::User(user) => ("user", Some(user), None),
    };
    tx.execute(
        "INSERT INTO activity_events (id, scope, network_id, at, actor_kind, actor_user_id, actor_node_id, actor_email, action, request_id, target_node_id, target_profile) VALUES (?1, 'network', ?2, ?3, ?4, ?5, ?6, (SELECT email FROM users WHERE id=?5), 'routing.select', ?7, ?8, ?9)",
        params![
            crate::accounts::random_uuid()?.as_bytes().as_slice(),
            network.as_uuid().as_bytes().as_slice(),
            now,
            kind,
            user.map(|u| u.as_uuid().as_bytes().to_vec()),
            actor_node.map(|n| n.as_uuid().as_bytes().to_vec()),
            request.as_uuid().as_bytes().as_slice(),
            node.as_uuid().as_bytes().as_slice(),
            profile,
        ],
    )?;
    let netmap = crate::store::netmap::recompile(
        tx,
        sealer,
        network,
        now,
        crate::store::netmap::Trigger::Node,
    )?;
    let selection = state(tx, network, &cfg, &bindings, &subjects, node)?;
    // The new netmaps change configuration progress and the inventory views.
    let mut effects = vec![Effect::Progress {
        network_id: network,
    }];
    effects.extend(netmap.effects());
    Ok((selection, effects))
}

pub(crate) fn select_profile(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request_id: RequestId,
    request: RoutingSelectionRequest,
) -> Result<(RoutingSelectionResponse, Vec<Effect>), Error> {
    let now = now()?;
    let p = authorize_node(tx, sealer, boot, token, now)?;
    current_instance(tx, boot, &p, &request.instance, now)?;
    let (selection, effects) = commit(
        tx,
        sealer,
        p.network,
        p.node,
        Actor::Node,
        request_id,
        Some(request.profile),
        Some(request.expected_version),
        now,
    )?;
    Ok((RoutingSelectionResponse { selection }, effects))
}

/// The Node clears its own selection under the same compare-and-swap base.
pub(crate) fn clear_profile(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request_id: RequestId,
    request: ClearRoutingSelectionRequest,
) -> Result<(RoutingSelectionResponse, Vec<Effect>), Error> {
    let now = now()?;
    let p = authorize_node(tx, sealer, boot, token, now)?;
    current_instance(tx, boot, &p, &request.instance, now)?;
    let (selection, effects) = commit(
        tx,
        sealer,
        p.network,
        p.node,
        Actor::Node,
        request_id,
        None,
        Some(request.expected_version),
        now,
    )?;
    Ok((RoutingSelectionResponse { selection }, effects))
}

/// A user may see and select a Node's profile when they manage the network
/// or the Node is their own User-identity device. Anything else, Tag Nodes
/// of a member included, is not found.
fn authorize_user(
    tx: &Transaction<'_>,
    token: &[u8; 32],
    network: NetworkId,
    cfg: &NetworkConfiguration,
    bindings: &ConfigurationBindings,
    node: NodeId,
    now: i64,
) -> Result<UserId, Error> {
    let user = crate::store::authorization::current_session(tx, token, now)?
        .user
        .id;
    let (member, role) = crate::store::authorization::membership(tx, network, user)?;
    let manager = matches!(role, MembershipRole::Owner | MembershipRole::Admin);
    if !cfg.nodes.contains_key(&node)
        || !(manager
            || user_binding(cfg, bindings, node)
                .is_some_and(|b| b.user_id == user && b.membership_id == member))
    {
        return Err(Error::NotFound);
    }
    Ok(user)
}

/// Whether a caller who may see the selection can also change it: only the
/// assignment's lock keeps everyone, managers included, on the default.
pub(crate) fn can_select(selection: &RoutingSelectionState) -> bool {
    !selection.locked
}

pub(crate) fn user_selection(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    token: [u8; 32],
    network: NetworkId,
    node: NodeId,
) -> Result<RoutingSelectionState, Error> {
    let (_, cfg, bindings) = current_configuration(tx, sealer, network)?;
    authorize_user(tx, &token, network, &cfg, &bindings, node, now()?)?;
    let subjects = subjects(tx, network, &bindings)?;
    state(tx, network, &cfg, &bindings, &subjects, node)
}

/// A user selects (`Some`) or clears (`None`) a Node's profile. The request
/// ID binds retries to the same Node and change.
pub(crate) fn select_user_profile(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    token: [u8; 32],
    network: NetworkId,
    node: NodeId,
    request_id: RequestId,
    profile: Option<String>,
) -> Result<(RoutingSelectionState, Vec<Effect>), Error> {
    let now = now()?;
    let (_, cfg, bindings) = current_configuration(tx, sealer, network)?;
    let user = authorize_user(tx, &token, network, &cfg, &bindings, node, now)?;
    let replay: Option<(Vec<u8>, String)> = tx
        .query_row(
            "SELECT target_node_id,target_profile FROM activity_events WHERE network_id=?1 AND actor_user_id=?2 AND request_id=?3 AND action='routing.select' AND actor_kind='user'",
            params![
                network.as_uuid().as_bytes().as_slice(),
                user.as_uuid().as_bytes().as_slice(),
                request_id.as_uuid().as_bytes().as_slice()
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((target, selected)) = replay {
        if id::<NodeId>(target)? != node || selected != profile.as_deref().unwrap_or(CLEARED) {
            return Err(Error::RequestConflict);
        }
        let subjects = subjects(tx, network, &bindings)?;
        return Ok((
            state(tx, network, &cfg, &bindings, &subjects, node)?,
            Vec::new(),
        ));
    }
    commit(
        tx,
        sealer,
        network,
        node,
        Actor::User(user),
        request_id,
        profile,
        None,
        now,
    )
}
