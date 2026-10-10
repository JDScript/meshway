//! Persistent DNS anchor, owned by the existing Kernel journal and monitor.
//! It has a local installation address, never a leased Server Mesh address.
use super::{Kernel, Pending, collect, execute, get_addresses, get_routes};
use crate::{Error, Result};
use meshway_dns::{DnsFuture, DnsLinkIdentity, OwnedDnsLink, SystemDnsError};
use rtnetlink::{
    Handle, LinkDummy,
    packet_route::{
        AddressFamily,
        address::{AddressAttribute, AddressMessage, AddressScope},
        link::{
            AfSpecInet6, AfSpecUnspec, In6AddrGenMode, InfoKind, LinkAttribute, LinkFlags,
            LinkInfo, LinkMessage,
        },
        route::{RouteAddress, RouteAttribute, RouteMessage},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub identity: DnsLinkIdentity,
    pub alias: String,
    pub hardware_address: Vec<u8>,
    pub mtu: u32,
}
struct Guard {
    // A separate request socket, used only by the read-only verifier below.
    // Cloning Kernel.handle would share its multipart-dump slot with mutations
    // and their verification. Cloning Arc<Guard> does not allocate another FD.
    handle: Handle,
    reads: tokio::sync::Mutex<()>,
    shared: Arc<super::super::protection::Shared>,
    record: Record,
    limit: usize,
    cancelled: CancellationToken,
}
impl Guard {
    fn check(&self) -> Result<()> {
        self.shared.check()?;
        if self.cancelled.is_cancelled() {
            return Err(Error::Conflict);
        }
        Ok(())
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        // This is a child of the anchor's token, never the Kernel's epoch.
        self.cancelled.cancel();
    }
}

// Dropping a response stream does not cancel the kernel's multipart dump. An
// interrupted read must close/poison this reader before another caller can use
// its FD. A caller cancelled while still waiting for the mutex owns no dump.
struct Reading(Option<CancellationToken>);
impl Drop for Reading {
    fn drop(&mut self) {
        if let Some(cancelled) = &self.0 {
            cancelled.cancel();
        }
    }
}
impl OwnedDnsLink for Guard {
    fn verify(&self) -> DnsFuture<'_, std::result::Result<DnsLinkIdentity, SystemDnsError>> {
        Box::pin(async {
            self.check().map_err(|_| SystemDnsError::Conflict)?;
            // GET execute() enqueues eagerly, so lock before invoking even the
            // first verifier helper, not inside collect(response_stream).
            let _read = tokio::select! { biased;
                _ = self.ended() => return Err(SystemDnsError::Conflict),
                read = self.reads.lock() => read,
            };
            self.check().map_err(|_| SystemDnsError::Conflict)?;
            let mut reading = Reading(Some(self.cancelled.clone()));
            tokio::select! { biased;
                _ = self.ended() => return Err(SystemDnsError::Conflict),
                result = verify(&self.handle, &self.record, self.limit) => result.map_err(|_| SystemDnsError::Conflict)?,
            }
            self.check().map_err(|_| SystemDnsError::Conflict)?;
            reading.0 = None;
            Ok(self.record.identity.clone())
        })
    }
    fn ended(&self) -> DnsFuture<'_, ()> {
        Box::pin(async {
            tokio::select! {
                _ = self.shared.conflict.cancelled() => {},
                _ = self.shared.tasks.stop.cancelled() => {},
                _ = self.cancelled.cancelled() => {},
            }
        })
    }
}
impl Kernel {
    /// Acquire/create the DNS anchor only after the namespace-wide DNS lock is
    /// held by the host installer. Existing records are verified, never adopted
    /// by name alone. DNS settings must be removed before remove_dns_anchor.
    pub async fn dns_anchor(
        &mut self,
        reservation: &meshway_dns::SystemDnsReservation,
    ) -> Result<Arc<dyn OwnedDnsLink>> {
        self.ready().await?;
        self.verify().await?;
        reservation
            .verify_namespace(
                &self.journal.boot,
                self.journal.namespace_device,
                self.journal.namespace_inode,
            )
            .map_err(|_| Error::Conflict)?;
        if !matches!(
            reservation.settings().backend,
            meshway_dns::SystemDnsBackend::Resolved
        ) {
            return Err(Error::Options);
        }
        let address = reservation.settings().anchor_address;
        if address.is_unspecified()
            || address.is_multicast()
            || address.is_loopback()
            || address.is_link_local()
            || address.is_broadcast()
            || self
                .journal
                .pools
                .iter()
                .any(|pool| pool.contains(&IpAddr::V4(address)))
        {
            return Err(Error::Conflict);
        }
        if let Some(record) = &self.journal.anchor {
            if record.identity.address != address {
                return Err(Error::Conflict);
            }
        } else {
            let name = format!("mwd{}", &self.journal.installation[..10]);
            let alias = format!("meshway-dns:{}", self.journal.installation);
            let links = collect(
                self.handle.link().get().execute(),
                self.options.max_kernel_objects,
            )
            .await?;
            if links.iter().any(|link| {
                link.attributes.iter().any(|a| {
                    matches!(a, LinkAttribute::IfName(v) if v == &name)
                        || matches!(a, LinkAttribute::IfAlias(v) if v == &alias)
                })
            }) {
                return Err(Error::Conflict);
            }
            collision(&self.handle, address, None, self.options.max_kernel_objects).await?;
            // IFLA_IFALIAS is not installed by the generic dummy NEWLINK path.
            // Give creation its own known fingerprint so the subsequent SETLINK
            // never adopts a same-name device with an unknown identity.
            let mut hardware_address = vec![0u8; 6];
            getrandom::fill(&mut hardware_address).map_err(|_| Error::Worker)?;
            hardware_address[0] = (hardware_address[0] & 0xfe) | 0x02;
            let mtu = 1500;
            self.journal.pending = Some(Pending::CreateDnsAnchor);
            self.store.persist(&self.journal).await?;
            // rtnl_create_link validates AF_SPEC before register_netdevice;
            // inet6_validate_link_af then rejects the not-yet-created inet6_dev
            // with EAFNOSUPPORT. Register DOWN first, without AF_SPEC or alias.
            // NETDEV_REGISTER creates inet6_dev; autoconfiguration waits for UP.
            let link = LinkDummy::new(&name)
                .address(hardware_address.clone())
                .mtu(mtu)
                .down()
                .build();
            execute(self.handle.link().add(link).execute()).await?;
            let links = collect(
                self.handle.link().get().match_name(name.clone()).execute(),
                self.options.max_kernel_objects,
            )
            .await?;
            if links.len() != 1 {
                return Err(Error::Conflict);
            }
            let index = links[0].header.index;
            let record = Record {
                identity: DnsLinkIdentity {
                    boot: self.journal.boot.trim().to_owned(),
                    namespace_device: self.journal.namespace_device,
                    namespace_inode: self.journal.namespace_inode,
                    installation: self.journal.installation.clone(),
                    index,
                    name,
                    address,
                },
                alias,
                hardware_address,
                mtu,
            };
            if index == 0
                || !device_matches(&record, &links[0])
                || links[0].header.flags.contains(LinkFlags::Up)
                || links[0]
                    .attributes
                    .iter()
                    .any(|a| matches!(a, LinkAttribute::IfAlias(value) if !value.is_empty()))
                || get_addresses(&self.handle, self.options.max_kernel_objects)
                    .await?
                    .iter()
                    .any(|address| address.header.index == index)
            {
                return Err(Error::Conflict);
            }
            // This separate SETLINK runs against a registered, fingerprinted,
            // still-DOWN device. Apply/read back both alias and addrgenmode
            // before any transition that could generate an IPv6 address.
            let configured = rtnetlink::LinkUnspec::new_with_index(index)
                .alias(record.alias.clone())
                .append_extra_attribute(LinkAttribute::AfSpecUnspec(vec![AfSpecUnspec::Inet6(
                    vec![AfSpecInet6::AddrGenMode(In6AddrGenMode::None)],
                )]))
                .build();
            execute(self.handle.link().set(configured).execute()).await?;
            let configured =
                owned_link(&self.handle, &record, self.options.max_kernel_objects).await?;
            if configured.header.flags.contains(LinkFlags::Up)
                || !installed_settings_match(&record, &configured)
            {
                return Err(Error::Conflict);
            }
            let mut request = self.handle.address().add(index, IpAddr::V4(address), 32);
            request.message_mut().header.scope = AddressScope::Universe;
            execute(request.execute()).await?;
            let configured =
                owned_link(&self.handle, &record, self.options.max_kernel_objects).await?;
            if configured.header.flags.contains(LinkFlags::Up)
                || !installed_settings_match(&record, &configured)
            {
                return Err(Error::Conflict);
            }
            verify_addresses(&self.handle, &record, self.options.max_kernel_objects).await?;
            execute(
                self.handle
                    .link()
                    .set(rtnetlink::LinkUnspec::new_with_index(index).up().build())
                    .execute(),
            )
            .await?;
            verify(&self.handle, &record, self.options.max_kernel_objects).await?;
            // Drain the creation's down/address/up notifications before arming
            // the steady-state watcher, then close the dump/watch race by a
            // second verification after arming it.
            self.quiesce().await?;
            self.journal.anchor = Some(record.clone());
            self.expected.lock().map_err(|_| Error::Worker)?.anchor = Some(record);
            verify(
                &self.handle,
                self.journal.anchor.as_ref().ok_or(Error::Journal)?,
                self.options.max_kernel_objects,
            )
            .await?;
            self.journal.pending = None;
            self.store.persist(&self.journal).await?;
            self.quiesce().await?;
            self.dns_cancel = CancellationToken::new();
        }
        let record = self.journal.anchor.clone().ok_or(Error::Journal)?;
        self.shared.check()?;
        let cancelled = self.dns_cancel.child_token();
        let (connection, handle, unsolicited) = rtnetlink::new_connection()?;
        drop(unsolicited); // no multicast subscriptions on this read socket
        let reader_stop = cancelled.clone();
        let conflict = self.shared.conflict.clone();
        // Tasks rejects at its existing capacity bound. On rejection the
        // supplied future (and its newly allocated socket) is dropped here.
        self.shared.tasks.spawn(async move {
            tokio::select! { biased;
                _ = reader_stop.cancelled() => {},
                _ = conflict.cancelled() => {},
                () = connection => { reader_stop.cancel(); },
            }
        })?;
        let guard = Guard {
            handle,
            reads: tokio::sync::Mutex::new(()),
            shared: self.shared.clone(),
            record,
            limit: self.options.max_kernel_objects,
            cancelled,
        };
        guard.check()?;
        Ok(Arc::new(guard))
    }

    /// Caller has restored the exact owned system-DNS settings first. Suspend
    /// and process exit deliberately never invoke this method.
    pub async fn remove_dns_anchor(&mut self) -> Result<()> {
        self.ready().await?;
        self.verify().await?;
        let Some(record) = self.journal.anchor.clone() else {
            return Ok(());
        };
        self.journal.pending = Some(Pending::DeleteDnsAnchor);
        self.store.persist(&self.journal).await?;
        self.dns_cancel.cancel();
        self.expected
            .lock()
            .map_err(|_| Error::Worker)?
            .anchor_deleting = true;
        execute(self.handle.link().del(record.identity.index).execute()).await?;
        let links = collect(
            self.handle.link().get().execute(),
            self.options.max_kernel_objects,
        )
        .await?;
        if links.iter().any(|link| {
            link.header.index == record.identity.index
                || link.attributes.iter().any(
                    |a| matches!(a, LinkAttribute::IfName(name) if name == &record.identity.name),
                )
        }) {
            return Err(Error::Conflict);
        }
        self.quiesce().await?;
        {
            let mut expected = self.expected.lock().map_err(|_| Error::Worker)?;
            expected.anchor = None;
            expected.anchor_deleting = false;
        }
        self.journal.anchor = None;
        self.journal.pending = None;
        self.store.persist(&self.journal).await
    }
}
pub(super) fn link_matches(record: &Record, link: &LinkMessage) -> bool {
    deleting_link_matches(record, link)
        && link
            .header
            .flags
            .contains(LinkFlags::Up | LinkFlags::LowerUp)
}

/// RTM_DELLINK first reports the same configured device becoming DOWN. Only
/// the journalled deletion window may accept that flag transition; its exact
/// identity, alias and address-generation configuration must still match.
pub(super) fn deleting_link_matches(record: &Record, link: &LinkMessage) -> bool {
    device_matches(record, link)
        && !link
            .header
            .flags
            .intersects(LinkFlags::Loopback | LinkFlags::Dormant)
        && installed_settings_match(record, link)
}
fn device_matches(record: &Record, link: &LinkMessage) -> bool {
    link.header.index == record.identity.index
        && link.attributes.iter().any(|a| matches!(a, LinkAttribute::Address(bytes) if bytes == &record.hardware_address))
        && link.attributes.iter().any(|a| matches!(a, LinkAttribute::Mtu(mtu) if *mtu == record.mtu))
        && !link.attributes.iter().any(|a| matches!(a, LinkAttribute::Controller(index) if *index != 0))
        && link.attributes.iter().any(|a| matches!(a, LinkAttribute::IfName(v) if v == &record.identity.name))
        && link.attributes.iter().any(|a| matches!(a, LinkAttribute::LinkInfo(infos) if infos.iter().any(|info| matches!(info, LinkInfo::Kind(InfoKind::Dummy)))))
}
fn installed_settings_match(record: &Record, link: &LinkMessage) -> bool {
    link.attributes
        .iter()
        .any(|a| matches!(a, LinkAttribute::IfAlias(value) if value == &record.alias))
        && link.attributes.iter().any(|a| {
            matches!(a,
            LinkAttribute::AfSpecUnspec(families) if families.iter().any(|family| matches!(family,
                AfSpecUnspec::Inet6(settings) if settings.iter().any(|setting| matches!(setting,
                    AfSpecInet6::AddrGenMode(In6AddrGenMode::None))))))
        })
}
pub(super) fn address_matches(record: &Record, address: &AddressMessage) -> bool {
    let flags = address
        .attributes
        .iter()
        .find_map(|a| match a {
            AddressAttribute::Flags(f) => Some(f.bits()),
            _ => None,
        })
        .unwrap_or(u32::from(address.header.flags.bits()));
    address.header.index == record.identity.index && address.header.family == AddressFamily::Inet
        && address.header.prefix_len == 32 && address.header.scope == AddressScope::Universe && flags & 0x68 == 0
        && address.attributes.iter().any(|a| matches!(a, AddressAttribute::Local(IpAddr::V4(ip)) if *ip == record.identity.address))
        && address.attributes.iter().all(|a| match a {
            AddressAttribute::Address(ip) | AddressAttribute::Local(ip) => *ip == IpAddr::V4(record.identity.address),
            _ => true,
        })
}
pub(super) fn route_conflicts(address: Ipv4Addr, owned: Option<u32>, route: &RouteMessage) -> bool {
    if route.header.address_family != AddressFamily::Inet
        || route.header.destination_prefix_length == 0
    {
        return false;
    }
    let Some(destination) = route.attributes.iter().find_map(|a| match a {
        RouteAttribute::Destination(RouteAddress::Inet(ip)) => Some(*ip),
        _ => None,
    }) else {
        return true;
    };
    let prefix = route.header.destination_prefix_length;
    if prefix > 32 {
        return true;
    }
    let mask = u32::MAX << (32 - prefix);
    if u32::from(destination) & mask != u32::from(address) & mask {
        return false;
    }
    !(prefix == 32
        && destination == address
        && route.header.protocol == rtnetlink::packet_route::route::RouteProtocol::Kernel
        && ((super::route_table(route) == 255
            && route.header.kind == rtnetlink::packet_route::route::RouteType::Local)
            || (super::route_table(route) == 254
                && route.header.kind == rtnetlink::packet_route::route::RouteType::Unicast))
        && owned.is_some_and(|index| {
            route
                .attributes
                .iter()
                .any(|a| matches!(a, RouteAttribute::Oif(i) if *i == index))
        }))
}
async fn collision(
    handle: &Handle,
    address: Ipv4Addr,
    owned: Option<u32>,
    limit: usize,
) -> Result<()> {
    let addresses = get_addresses(handle, limit).await?;
    if addresses.iter().any(|entry| Some(entry.header.index) != owned && entry.attributes.iter().any(|a| matches!(a, AddressAttribute::Address(IpAddr::V4(ip)) | AddressAttribute::Local(IpAddr::V4(ip)) if *ip == address))) { return Err(Error::Conflict); }
    if get_routes(handle, limit)
        .await?
        .iter()
        .any(|route| route_conflicts(address, owned, route))
    {
        return Err(Error::Conflict);
    }
    Ok(())
}
pub(super) async fn verify(handle: &Handle, record: &Record, limit: usize) -> Result<()> {
    let identity = super::journal::Journal::identity()?;
    if identity.0.trim() != record.identity.boot
        || identity.1 != record.identity.namespace_device
        || identity.2 != record.identity.namespace_inode
    {
        return Err(Error::Conflict);
    }
    let link = owned_link(handle, record, limit).await?;
    if !link_matches(record, &link) {
        return Err(Error::Conflict);
    }
    verify_addresses(handle, record, limit).await?;
    collision(
        handle,
        record.identity.address,
        Some(record.identity.index),
        limit,
    )
    .await
}

async fn owned_link(handle: &Handle, record: &Record, limit: usize) -> Result<LinkMessage> {
    let links = collect(handle.link().get().execute(), limit).await?;
    let matches: Vec<_> = links
        .iter()
        .filter(|link| {
            link.header.index == record.identity.index
                || link.attributes.iter().any(|a| {
                    matches!(a, LinkAttribute::IfName(v) if v == &record.identity.name)
                        || matches!(a, LinkAttribute::IfAlias(v) if v == &record.alias)
                })
        })
        .collect();
    if matches.len() != 1 || !device_matches(record, matches[0]) {
        return Err(Error::Conflict);
    }
    Ok(matches[0].clone())
}

async fn verify_addresses(handle: &Handle, record: &Record, limit: usize) -> Result<()> {
    let addresses = get_addresses(handle, limit).await?;
    let owned: Vec<_> = addresses
        .iter()
        .filter(|address| address.header.index == record.identity.index)
        .collect();
    if owned.len() != 1 || !address_matches(record, owned[0]) {
        return Err(Error::Conflict);
    }
    Ok(())
}
