use super::*;
use crate::{
    accounts::issue_one_time,
    networks::management::{CreatedLink, LinkInspection, LinkPage},
};
use meshway_model::{JoinLinkId, MembershipId, MembershipRole, Version};
use meshway_protocol::management::JoinLinkView;
use sha2::{Digest as _, Sha256};

use crate::store::authorization::{
    membership, owner_count, parse_role, protect_owner, require_manager,
};
fn role_name(role: MembershipRole) -> &'static str {
    match role {
        MembershipRole::Owner => "owner",
        MembershipRole::Admin => "admin",
        MembershipRole::Member => "member",
    }
}
fn grant(issuer: MembershipRole, role: MembershipRole) -> bool {
    crate::store::authorization::can_grant(issuer, role)
}

fn removal(
    tx: &Transaction<'_>,
    user: UserId,
    network: NetworkId,
    member: Option<MembershipId>,
    request: RequestId,
    sealer: &Sealer,
    cache: &crate::configuration::SnapshotCache,
) -> Result<meshway_protocol::management::RemovalImpactResponse, Error> {
    let (own, role) = membership(tx, network, user)?;
    let target = member.unwrap_or(own);
    let (target_user, target_role, version) = tx.query_row(
        "SELECT user_id,role,version FROM memberships WHERE network_id=?1 AND id=?2 AND revoked_at IS NULL",
        params![network.as_uuid().as_bytes().as_slice(), target.as_uuid().as_bytes().as_slice()],
        |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
    ).optional()?.ok_or(Error::NotFound)?;
    let target_role = parse_role(&target_role)?;
    if target != own && !grant(role, target_role) {
        return Err(Error::PermissionDenied);
    }
    let target_user = UserId::new(
        uuid::Uuid::from_slice(&target_user).map_err(|_| Error::decode("memberships.user_id"))?,
    )
    .map_err(|_| Error::decode("memberships.user_id"))?;
    let revision = meshway_model::Revision::new(
        u64::try_from(tx.query_row(
            "SELECT current_revision FROM networks WHERE id=?1",
            [network.as_uuid().as_bytes().as_slice()],
            |r| r.get::<_, i64>(0),
        )?)
        .map_err(|_| Error::decode("networks.current_revision"))?,
    )
    .map_err(|_| Error::decode("networks.current_revision"))?;
    let snapshot = cache.load(tx, network, revision, sealer)?;
    let node_ids: Vec<_> = snapshot
        .document
        .configuration
        .nodes
        .iter()
        .filter(|(id, n)| match &n.identity {
            meshway_configuration::NodeIdentity::User { .. } => {
                snapshot.bindings.node_users.get(id).is_some_and(|binding| {
                    binding.user_id == target_user && binding.membership_id == target
                })
            }
            meshway_configuration::NodeIdentity::Tag { .. } => false,
        })
        .map(|(id, _)| *id)
        .collect();
    Ok(meshway_protocol::management::RemovalImpactResponse {
        request_id: request,
        membership_id: target,
        version: Version::new(
            u64::try_from(version).map_err(|_| Error::decode("memberships.version"))?,
        )
        .map_err(|_| Error::decode("memberships.version"))?,
        revision,
        node_count: u32::try_from(node_ids.len())
            .map_err(|_| Error::Internal("removal node count overflow"))?,
        node_ids,
    })
}

pub(crate) fn removal_impact(
    db: &mut Connection,
    digest: [u8; 32],
    network: NetworkId,
    member: Option<MembershipId>,
    request: RequestId,
    sealer: &Sealer,
    cache: &crate::configuration::SnapshotCache,
) -> Result<meshway_protocol::management::RemovalImpactResponse, Error> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let user = current_session(&tx, &digest, now()?)?.user.id;
    let result = removal(&tx, user, network, member, request, sealer, cache)?;
    tx.commit()?;
    Ok(result)
}

pub(crate) fn remove_member(
    db: &mut Connection,
    digest: [u8; 32],
    command: crate::networks::management::MemberRemoval,
    sealer: &Sealer,
    cache: &crate::configuration::SnapshotCache,
    publisher: &crate::state::service::CommitPublisher,
) -> Result<meshway_model::Revision, Error> {
    let crate::networks::management::MemberRemoval {
        network,
        member,
        expected,
        revision,
        request,
    } = command;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let user = current_session(&tx, &digest, now)?.user.id;
    let impact = removal(&tx, user, network, member, request, sealer, cache)?;
    if impact.revision != revision {
        return Err(Error::RevisionConflict);
    }
    if impact.version != expected {
        return Err(Error::VersionConflict);
    }
    let role = tx.query_row(
        "SELECT role FROM memberships WHERE id=?1",
        [impact.membership_id.as_uuid().as_bytes().as_slice()],
        |r| r.get::<_, String>(0),
    )?;
    if role == "owner" {
        protect_owner(&tx, network)?;
    }
    let effects = crate::execution::membership::remove_nodes(
        &tx,
        &crate::execution::membership::MembershipRemoval {
            digest,
            network,
            membership: impact.membership_id,
            version: expected,
            request,
        },
        sealer,
        cache,
    )?;
    tx.execute(
        "UPDATE memberships SET revoked_at=?1,version=version+1 WHERE id=?2",
        params![now, impact.membership_id.as_uuid().as_bytes().as_slice()],
    )?;
    tx.execute("UPDATE join_links SET revoked_at=?1 WHERE issuer_membership_id=?2 AND consumed_at IS NULL AND revoked_at IS NULL",params![now,impact.membership_id.as_uuid().as_bytes().as_slice()])?;
    record(
        &tx,
        network,
        user,
        request,
        Activity::MemberRemoved(impact.membership_id),
        now,
    )?;
    let mut changes = effects.changes();
    if effects.notice.is_none() {
        changes.push(crate::state::stream::CommittedChange::Authorization {
            network_id: network,
        });
    }
    tx.commit()?;
    for change in changes {
        publisher.publish_committed(change);
    }
    Ok(effects.revision)
}

pub(crate) fn change_role(
    db: &mut Connection,
    digest: [u8; 32],
    network: NetworkId,
    id: MembershipId,
    expected: Version,
    role: MembershipRole,
    request: RequestId,
) -> Result<(crate::networks::MemberSummary, bool), Error> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let user = current_session(&tx, &digest, now)?.user.id;
    let caller = membership(&tx, network, user)?;
    let raw = tx.query_row(
        "SELECT m.sequence,m.id,m.user_id,u.email,m.role,m.version,m.joined_at FROM memberships m JOIN users u ON u.id=m.user_id WHERE m.network_id=?1 AND m.id=?2 AND m.revoked_at IS NULL",
        params![network.as_uuid().as_bytes().as_slice(), id.as_uuid().as_bytes().as_slice()],
        RawMember::read,
    ).optional()?.ok_or(Error::NotFound)?;
    let owners = owner_count(&tx, network)?;
    let mut member = raw.into_view(caller, owners)?;
    if !grant(caller.1, member.role) || !grant(caller.1, role) {
        return Err(Error::PermissionDenied);
    }
    if member.version != expected {
        return Err(Error::VersionConflict);
    }
    if !member.actions.assignable_roles.contains(&role) {
        return Err(if member.role == MembershipRole::Owner && owners <= 1 {
            Error::LastOwner
        } else {
            Error::PermissionDenied
        });
    }
    let changed = member.role != role;
    if changed {
        if member.role == MembershipRole::Owner {
            protect_owner(&tx, network)?;
        }
        let version = expected
            .checked_next()
            .map_err(|_| Error::Internal("membership version overflow"))?;
        tx.execute(
            "UPDATE memberships SET role=?1,version=?2 WHERE network_id=?3 AND id=?4",
            params![
                role_name(role),
                version.get() as i64,
                network.as_uuid().as_bytes().as_slice(),
                id.as_uuid().as_bytes().as_slice()
            ],
        )?;
        // Permission loss is irreversible for existing links, including later
        // promotion or leaving/rejoining under a new membership.
        tx.execute(
            "UPDATE join_links SET revoked_at=?1 WHERE issuer_membership_id=?2 AND consumed_at IS NULL AND revoked_at IS NULL AND (?3='member' OR (?3='admin' AND role!='member'))",
            params![now, id.as_uuid().as_bytes().as_slice(), role_name(role)],
        )?;
        record(
            &tx,
            network,
            user,
            request,
            Activity::MemberRoleChanged(id),
            now,
        )?;
        member.role = role;
        member.version = version;
    }
    member.actions = crate::store::authorization::member_actions(
        membership(&tx, network, user)?,
        (member.membership_id, member.role),
        owner_count(&tx, network)?,
    );
    tx.commit()?;
    Ok((member, changed))
}

struct Link {
    sequence: i64,
    view: JoinLinkView,
    issuer: MembershipId,
    created_by: UserId,
    consumed_member: Option<MembershipId>,
}
const LINK_SELECT: &str = "SELECT sequence,id,network_id,role,created_at,expires_at,revoked_at,consumed_at,consumed_by_user_id,issuer_membership_id,created_by,consumed_membership_id FROM join_links";
fn read_link(r: &rusqlite::Row<'_>) -> rusqlite::Result<Link> {
    fn id<T>(
        r: &rusqlite::Row<'_>,
        index: usize,
        make: impl FnOnce(uuid::Uuid) -> Result<T, meshway_model::InvalidId>,
    ) -> rusqlite::Result<T> {
        let bytes = r.get::<_, Vec<u8>>(index)?;
        let u = uuid::Uuid::from_slice(&bytes).map_err(|_| rusqlite::Error::InvalidQuery)?;
        make(u).map_err(|_| rusqlite::Error::InvalidQuery)
    }
    fn optional_id<T>(
        r: &rusqlite::Row<'_>,
        index: usize,
        make: impl FnOnce(uuid::Uuid) -> Result<T, meshway_model::InvalidId>,
    ) -> rusqlite::Result<Option<T>> {
        if r.get::<_, Option<Vec<u8>>>(index)?.is_none() {
            Ok(None)
        } else {
            id(r, index, make).map(Some)
        }
    }
    fn time(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<chrono::DateTime<Utc>> {
        chrono::DateTime::from_timestamp_millis(r.get(i)?).ok_or(rusqlite::Error::InvalidQuery)
    }
    fn opttime(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<Option<chrono::DateTime<Utc>>> {
        r.get::<_, Option<i64>>(i)?
            .map(|v| {
                chrono::DateTime::from_timestamp_millis(v).ok_or(rusqlite::Error::InvalidQuery)
            })
            .transpose()
    }
    Ok(Link {
        sequence: r.get(0)?,
        view: JoinLinkView {
            id: id(r, 1, JoinLinkId::new)?,
            network_id: id(r, 2, NetworkId::new)?,
            role: parse_role(&r.get::<_, String>(3)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
            created_at: time(r, 4)?,
            expires_at: time(r, 5)?,
            revoked_at: opttime(r, 6)?,
            consumed_at: opttime(r, 7)?,
            consumed_by_user_id: optional_id(r, 8, UserId::new)?,
            actions: meshway_protocol::management::JoinLinkActions { revoke: false },
        },
        issuer: id(r, 9, MembershipId::new)?,
        created_by: id(r, 10, UserId::new)?,
        consumed_member: optional_id(r, 11, MembershipId::new)?,
    })
}
fn valid(tx: &Transaction<'_>, link: &Link, now: i64) -> Result<(), Error> {
    let (id, role) =
        membership(tx, link.view.network_id, link.created_by).map_err(|error| match error {
            Error::NotFound => Error::LinkIssuerUnavailable,
            other => other,
        })?;
    if id != link.issuer || !grant(role, link.view.role) {
        return Err(Error::LinkIssuerUnavailable);
    }
    if link.view.revoked_at.is_some() {
        return Err(Error::LinkRevoked);
    }
    if link.view.expires_at.timestamp_millis() <= now {
        return Err(Error::LinkExpired);
    }
    Ok(())
}
fn link_actions(link: &mut Link, role: Option<MembershipRole>, now: i64) {
    link.view.actions.revoke = role.is_some_and(|role| grant(role, link.view.role))
        && link.view.revoked_at.is_none()
        && link.view.consumed_at.is_none()
        && link.view.expires_at.timestamp_millis() > now;
}
pub(crate) fn create_link(
    db: &mut Connection,
    digest: [u8; 32],
    network: NetworkId,
    role: MembershipRole,
    ttl: u32,
    request: RequestId,
) -> Result<CreatedLink, Error> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let user = current_session(&tx, &digest, now)?.user.id;
    let (issuer, issuer_role) = membership(&tx, network, user)?;
    if !grant(issuer_role, role) {
        return Err(Error::PermissionDenied);
    }
    let input_digest = Sha256::digest(format!("join-link/v1\0{}\0{ttl}", role_name(role)));
    let previous = tx.query_row(
        "SELECT creation_request_digest FROM join_links WHERE network_id=?1 AND created_by=?2 AND creation_request_id=?3",
        params![network.as_uuid().as_bytes().as_slice(), user.as_uuid().as_bytes().as_slice(), request.as_uuid().as_bytes().as_slice()],
        |row| row.get::<_, Vec<u8>>(0),
    ).optional()?;
    if let Some(previous) = previous {
        if previous.as_slice() != input_digest.as_slice() {
            return Err(Error::RequestConflict);
        }
        let mut link = tx.query_row(
            &format!(
                "{LINK_SELECT} WHERE network_id=?1 AND created_by=?2 AND creation_request_id=?3"
            ),
            params![
                network.as_uuid().as_bytes().as_slice(),
                user.as_uuid().as_bytes().as_slice(),
                request.as_uuid().as_bytes().as_slice()
            ],
            read_link,
        )?;
        link_actions(&mut link, Some(issuer_role), now);
        tx.commit()?;
        return Ok(CreatedLink {
            link: link.view,
            secret: None,
        });
    }
    let id =
        JoinLinkId::new(random_uuid()?).map_err(|_| Error::Internal("generated join link ID"))?;
    let (secret, hash) = issue_one_time("mwj_")?;
    let expires = now
        .checked_add(i64::from(ttl) * 1000)
        .ok_or(Error::Internal("join link expiry overflow"))?;
    tx.execute(
        "INSERT INTO join_links(id,network_id,role,token_hash,created_by,issuer_membership_id,created_at,expires_at,creation_request_id,creation_request_digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![id.as_uuid().as_bytes().as_slice(), network.as_uuid().as_bytes().as_slice(), role_name(role),
            hash.as_slice(), user.as_uuid().as_bytes().as_slice(), issuer.as_uuid().as_bytes().as_slice(),
            now, expires, request.as_uuid().as_bytes().as_slice(), input_digest.as_slice()],
    )?;
    record(&tx, network, user, request, Activity::LinkCreated(id), now)?;
    let mut link = tx.query_row(
        &format!("{LINK_SELECT} WHERE id=?1"),
        [id.as_uuid().as_bytes().as_slice()],
        read_link,
    )?;
    link_actions(&mut link, Some(issuer_role), now);
    tx.commit()?;
    Ok(CreatedLink {
        link: link.view,
        secret: Some(secret),
    })
}
pub(crate) fn inspect_link(
    db: &mut Connection,
    digest: [u8; 32],
    hash: [u8; 32],
) -> Result<LinkInspection, Error> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let now = now()?;
    let user = current_session(&tx, &digest, now)?.user.id;
    let mut link = tx
        .query_row(
            &format!("{LINK_SELECT} WHERE token_hash=?1"),
            [hash.as_slice()],
            read_link,
        )
        .optional()?
        .ok_or(Error::InvalidLink)?;
    if link.view.consumed_at.is_some() {
        return Err(Error::LinkUsed);
    }
    valid(&tx, &link, now)?;
    let member = membership(&tx, link.view.network_id, user).ok();
    link_actions(&mut link, member.map(|(_, role)| role), now);
    let name = tx.query_row(
        "SELECT name FROM networks WHERE id=?1",
        [link.view.network_id.as_uuid().as_bytes().as_slice()],
        |r| r.get(0),
    )?;
    tx.commit()?;
    Ok(LinkInspection {
        link: link.view,
        network_name: name,
        already_member: member.is_some(),
    })
}
pub(crate) fn redeem_link(
    db: &mut Connection,
    digest: [u8; 32],
    hash: [u8; 32],
    request: RequestId,
) -> Result<((NetworkId, MembershipId), bool), Error> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let user = current_session(&tx, &digest, now)?.user.id;
    let link = tx
        .query_row(
            &format!("{LINK_SELECT} WHERE token_hash=?1"),
            [hash.as_slice()],
            read_link,
        )
        .optional()?
        .ok_or(Error::InvalidLink)?;
    if let Some(member) = link.consumed_member {
        if link.view.consumed_by_user_id != Some(user)
            || membership(&tx, link.view.network_id, user)
                .ok()
                .map(|m| m.0)
                != Some(member)
        {
            return Err(Error::LinkUsed);
        }
        tx.commit()?;
        return Ok(((link.view.network_id, member), false));
    }
    valid(&tx, &link, now)?;
    if membership(&tx, link.view.network_id, user).is_ok() {
        return Err(Error::AlreadyMember);
    }
    let member = MembershipId::new(random_uuid()?)
        .map_err(|_| Error::Internal("generated membership ID"))?;
    tx.execute(
        "INSERT INTO memberships(id,network_id,user_id,role,version,joined_at) VALUES(?1,?2,?3,?4,1,?5)",
        params![member.as_uuid().as_bytes().as_slice(), link.view.network_id.as_uuid().as_bytes().as_slice(),
            user.as_uuid().as_bytes().as_slice(), role_name(link.view.role), now],
    )?;
    tx.execute(
        "UPDATE join_links SET consumed_at=?1,consumed_by_user_id=?2,consumed_membership_id=?3 WHERE id=?4",
        params![now, user.as_uuid().as_bytes().as_slice(), member.as_uuid().as_bytes().as_slice(), link.view.id.as_uuid().as_bytes().as_slice()],
    )?;
    record(
        &tx,
        link.view.network_id,
        user,
        request,
        Activity::MemberJoined {
            member,
            link: link.view.id,
        },
        now,
    )?;
    tx.commit()?;
    Ok(((link.view.network_id, member), true))
}
pub(crate) fn revoke_link(
    db: &mut Connection,
    digest: [u8; 32],
    network: NetworkId,
    id: JoinLinkId,
    request: RequestId,
) -> Result<(), Error> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let user = current_session(&tx, &digest, now)?.user.id;
    let (_, role) = membership(&tx, network, user)?;
    require_manager(role)?;
    let link = tx
        .query_row(
            &format!("{LINK_SELECT} WHERE id=?1 AND network_id=?2"),
            params![
                id.as_uuid().as_bytes().as_slice(),
                network.as_uuid().as_bytes().as_slice()
            ],
            read_link,
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    if !grant(role, link.view.role) {
        return Err(Error::PermissionDenied);
    }
    let count=tx.execute("UPDATE join_links SET revoked_at=?1 WHERE id=?2 AND revoked_at IS NULL AND consumed_at IS NULL AND expires_at>?1",params![now,id.as_uuid().as_bytes().as_slice()])?;
    if count > 0 {
        record(&tx, network, user, request, Activity::LinkRevoked(id), now)?;
    }
    tx.commit()?;
    Ok(())
}
pub(crate) fn links(
    db: &mut Connection,
    digest: [u8; 32],
    network: NetworkId,
    limit: u16,
    cursor: Option<String>,
    sealer: &Sealer,
) -> Result<LinkPage, Error> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let now = now()?;
    let user = current_session(&tx, &digest, now)?.user.id;
    let role = membership(&tx, network, user)?.1;
    require_manager(role)?;
    let scope = CursorScope::JoinLinks { user, network };
    let mut window = match cursor {
        Some(cursor) => Window::decode(sealer, scope, &cursor, now)?,
        None => Window::new(ceiling(&tx, "join_links")?, 0, now)?,
    };
    let total: i64 = tx.query_row(
        "SELECT count(*) FROM join_links WHERE network_id=?1 AND sequence<=?2",
        params![network.as_uuid().as_bytes().as_slice(), window.upper],
        |row| row.get(0),
    )?;
    let mut stmt=tx.prepare(&format!("{LINK_SELECT} WHERE network_id=?1 AND sequence<=?2 AND sequence<=?3 ORDER BY sequence DESC LIMIT ?4"))?;
    let mut links = stmt
        .query_map(
            params![
                network.as_uuid().as_bytes().as_slice(),
                window.upper - window.last,
                window.upper,
                i64::from(limit) + 1
            ],
            read_link,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let more = links.len() > usize::from(limit);
    links.truncate(usize::from(limit));
    let items = links
        .into_iter()
        .map(|mut link| {
            window.last = window.upper - link.sequence + 1;
            link_actions(&mut link, Some(role), now);
            link.view
        })
        .collect();
    let next_cursor = if more {
        Some(window.encode(sealer, scope)?)
    } else {
        None
    };
    let as_of = window.as_of()?;
    drop(stmt);
    tx.commit()?;
    Ok(LinkPage {
        total: meshway_model::Count::new(
            total
                .try_into()
                .map_err(|_| Error::decode("join_links count"))?,
        )
        .map_err(|_| Error::decode("join_links count"))?,
        items,
        next_cursor,
        as_of,
    })
}
