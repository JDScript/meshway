//! Opening one term of kernel ownership: recover and verify what the
//! journals record, and allocate the capture block from the live tables.
use super::proof::verify_rules;
use super::*;

impl Kernel {
    pub async fn open(directory: Arc<Directory>, options: Options, tasks: Tasks) -> Result<Self> {
        Self::open_term(directory, options, tasks, Default::default()).await
    }

    /// Open one term of kernel ownership: recover and verify everything the
    /// journals record, exactly as at process start. A Host that proves its
    /// ownership again after an unavailable end opens its next term here,
    /// continuing what the ended term published.
    pub async fn open_term(
        directory: Arc<Directory>,
        options: Options,
        tasks: Tasks,
        published: super::super::protection::Published,
    ) -> Result<Self> {
        // Subscribe before the baseline dump so rule changes cannot disappear
        // between discovery and the monitor's start.
        let (connection, handle) = rtnl::connect()?;
        let conflict = CancellationToken::new();
        let ending = Arc::new(super::super::protection::Ending::default());
        // Set once this term opened; a connection that ends before (a failed
        // open dropping its handles) or after its Kernel is dropped is no end.
        let term = Arc::new(std::sync::OnceLock::<std::sync::Weak<()>>::new());
        // A request connection that stops while its Kernel lives (its socket
        // failed) ends the term as unavailable: nothing was read, and a
        // re-proof opens new ones.
        let connection_ended = |name: &'static str| {
            let (conflict, ending, term) = (conflict.clone(), ending.clone(), term.clone());
            move || {
                if term.get().and_then(std::sync::Weak::upgrade).is_none() {
                    return;
                }
                tracing::warn!(
                    connection = name,
                    "Kernel netlink connection ended; kernel ownership ends"
                );
                ending.end(
                    &conflict,
                    crate::OwnershipEnd::Unavailable {
                        check: "connection_unavailable",
                    },
                );
            }
        };
        let failed = connection_ended("owner");
        tasks.spawn(async move {
            connection.await;
            failed();
        })?;
        exit::recovery::recover(&handle, directory.clone(), options.max_kernel_objects).await?;
        egress::recover(
            &handle,
            directory.clone(),
            options.max_kernel_objects,
            false,
        )
        .await?;
        let monitor_socket = monitor::open()?;
        // Observation (change fingerprints, route lookups) has its own
        // request socket: it never queues behind or interleaves with the
        // owner's mutations and their readback dumps.
        let (observer_connection, observer) = rtnl::connect()?;
        let observer_failed = connection_ended("observer");
        tasks.spawn(async move {
            observer_connection.await;
            observer_failed();
        })?;
        let (mut store, mut previous) = Store::open(directory).await?;
        let rules = get_rules(&handle, options.max_kernel_objects).await?;
        let routes = get_routes(&handle, options.max_kernel_objects).await?;
        // The same boot and nsfs inode but none of the journal's objects: a
        // new namespace reused the inode (container stop/start). Nothing here
        // is ours to adopt or clean up; a partial set stays a conflict below.
        if let Some(journal) = previous
            .as_ref()
            .filter(|journal| !journal.rules.is_empty() || journal.anchor.is_some())
        {
            let observed: Vec<_> = rules.iter().cloned().map(encode_rule).collect();
            let aliases = link_aliases(&handle, options.max_kernel_objects).await?;
            if !journal.resident(&observed, &aliases) {
                store.archive_foreign(journal).await?;
                previous = None;
            }
        }
        let mut journal = match previous {
            Some(journal)
                if !journal.rules.is_empty()
                    || !journal.routes.is_empty()
                    || journal.anchor.is_some() =>
            {
                journal
            }
            _ => allocate(&rules, &routes, options.max_pools)?,
        };
        if journal.pool_limit != options.max_pools {
            return Err(Error::Options);
        }
        verify_rules(&journal, &rules)?;
        // A previous process's nonpersistent TUN is never adopted. Any live
        // positive route in its private table is ambiguous and blocks recovery.
        if routes
            .iter()
            .any(|route| route_table(route) == journal.table)
        {
            return Err(Error::Conflict);
        }
        journal.routes.clear();
        journal.tun = None;
        let mut expected = Expected {
            anchor: journal.anchor.clone(),
            pools: journal.pools.clone(),
            own_rules: journal
                .rules
                .iter()
                .map(|record| record.observed.clone())
                .collect(),
            ..Expected::default()
        };
        // An external address inside a retained pool is the overlap as it
        // is now (the monitor's rule): this term starts failed closed on it
        // and recovers on its departure. Any other pool object conflicts.
        let addresses = get_addresses(&handle, options.max_kernel_objects).await?;
        for address in &addresses {
            if capture::address_conflicts(&journal.pools, None, address) {
                if !monitor::external(&expected, address.header.index) {
                    return Err(Error::Conflict);
                }
                let overlapping = monitor::pool_addresses(&expected, address);
                expected.overlaps.extend(overlapping);
            }
        }
        if routes.iter().any(|route| {
            capture::route_conflicts(&journal.pools, None, route)
                && !monitor::overlap_route(&expected, route)
        }) {
            return Err(Error::Conflict);
        }
        let foreign: Vec<_> = rules
            .into_iter()
            .filter(|rule| {
                !journal
                    .rules
                    .iter()
                    .any(|record| record.observed == encode_rule(rule.clone()))
            })
            .collect();
        let selectors = plan::selectors(&foreign)?;
        journal.mark.transform(0, &selectors)?;
        let mut baseline: Vec<_> = foreign.into_iter().map(encode_rule).collect();
        baseline.sort();
        let bootstrap_sockets = (options.max_sockets / 8).clamp(1, 16);
        let super::super::protection::Published {
            mut network,
            changes,
            cleanups,
        } = published;
        network.routes = Some(observer.clone());
        let shared = Arc::new(Shared {
            resources: super::super::protection::Resources::new(options.max_sockets * 4),
            mark: journal.mark,
            selectors,
            conflict,
            ending,
            network,
            tasks: tasks.clone(),
            sockets: Arc::new(Semaphore::new(bootstrap_sockets)),
            data_sockets: Arc::new(Semaphore::new(options.max_sockets - bootstrap_sockets)),
            attempts: Arc::new(Semaphore::new(options.max_sockets)),
            resolutions: Arc::new(Semaphore::new(options.max_sockets)),
            execution: RwLock::new(None),
            changes,
            cleanups,
        });
        // Permissions and readback are real even before the first connection.
        for socket in [
            tokio::net::TcpSocket::new_v4()?,
            tokio::net::TcpSocket::new_v6()?,
        ] {
            shared.prepare(&socket2::SockRef::from(&socket))?;
        }
        if let Some(anchor) = &journal.anchor {
            // Recovery above left no route in a source-capture table.
            anchor::verify(&handle, anchor, None, options.max_kernel_objects).await?;
        }
        // A new term publishes the overlap it read, replacing what an ended
        // term reported.
        let overlap = expected.overlap();
        shared.network.overlap.send_if_modified(|current| {
            let changed = *current != overlap;
            *current = overlap;
            changed
        });
        let expected = Arc::new(Mutex::new(expected));
        let watched = shared.clone();
        let events = expected.clone();
        let table = journal.table;
        tasks.spawn(async move {
            monitor::run(monitor_socket, watched, events, table).await;
        })?;
        tasks.spawn(network::classify(
            shared.clone(),
            observer,
            expected.clone(),
            options.max_kernel_objects,
        ))?;
        store.persist(&journal).await?;
        shared.check()?;
        let alive = Arc::new(());
        let _ = term.set(Arc::downgrade(&alive));
        Ok(Self {
            _term: alive,
            handle,
            store,
            journal,
            shared,
            expected,
            tun: None,
            options,
            baseline,
            dns_cancel: CancellationToken::new(),
            exit: None,
            probe: None,
            egress: None,
            clamp: None,
            repaired: std::collections::VecDeque::new(),
            repair_owed: None,
        })
    }
}

/// Interface aliases of this namespace.
async fn link_aliases(handle: &Handle, limit: usize) -> Result<Vec<String>> {
    Ok(get_links(handle, limit)
        .await?
        .into_iter()
        .flat_map(|link| link.attributes)
        .filter_map(|attribute| match attribute {
            rtnetlink::packet_route::link::LinkAttribute::IfAlias(alias) => Some(alias),
            _ => None,
        })
        .collect())
}

pub(super) fn allocate(
    rules: &[RuleMessage],
    routes: &[RouteMessage],
    pool_limit: usize,
) -> Result<Journal> {
    let selectors = plan::selectors(rules)?;
    // Only the ordinary local lookup may precede our capture block.
    if rules.iter().filter(|r| priority(r) == 0).any(|rule| {
        rule_table(rule) != 255
            || rule.header.action != rtnetlink::packet_route::rule::RuleAction::ToTable
            || rule.header.src_len != 0
            || rule.header.dst_len != 0
            || rule.header.tos != 0
            || !rule.header.flags.is_empty()
            || rule.attributes.iter().any(|a| {
                !matches!(
                    a,
                    RuleAttribute::Table(_)
                        | RuleAttribute::Priority(_)
                        | RuleAttribute::Protocol(_)
                        | RuleAttribute::SuppressPrefixLen(u32::MAX)
                        | RuleAttribute::SuppressIfGroup(u32::MAX)
                )
            })
    }) {
        return Err(Error::Conflict);
    }
    for family in [AddressFamily::Inet, AddressFamily::Inet6] {
        if rules
            .iter()
            .filter(|r| r.header.family == family && priority(r) == 0)
            .count()
            != 1
        {
            return Err(Error::Conflict);
        }
    }
    let first = rules
        .iter()
        .map(priority)
        .filter(|p| *p > 0)
        .min()
        .unwrap_or(32766)
        .min(32766);
    let width = pool_limit as u32 * 2 + 2 + 32;
    let reserved = exit::routing::RESERVED;
    if first <= width + reserved {
        return Err(Error::Conflict);
    }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| Error::Worker)?;
    let seed = u32::from_ne_bytes(random[..4].try_into().map_err(|_| Error::Worker)?);
    let start = (0..1024)
        .find_map(|attempt| {
            let start = 1 + reserved + seed.wrapping_add(attempt) % (first - width - reserved);
            let end = start + width - 1;
            (!rules.iter().any(|rule| {
                rule.attributes.iter().any(
                    |a| matches!(a, RuleAttribute::Goto(target) if (start..=end).contains(target)),
                )
            }))
            .then_some(start)
        })
        .ok_or(Error::Conflict)?;
    let table = (0..1024)
        .map(|offset| 10000 + seed.wrapping_add(offset) % (u32::MAX - 20000))
        .find(|table| {
            !rules.iter().any(|rule| rule_table(rule) == *table)
                && !routes.iter().any(|route| route_table(route) == *table)
        })
        .ok_or(Error::Conflict)?;
    let mark = (0..32)
        .map(|shift| {
            let bit = 1u32 << ((shift + seed % 32) % 32);
            Mark {
                value: bit,
                mask: bit,
            }
        })
        .find(|mark| mark.transform(0, &selectors).is_ok())
        .ok_or(Error::Conflict)?;
    let namespace = namespace::Identity::current()?;
    let installation = random.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(Journal {
        version: 1,
        boot: namespace.boot,
        namespace_device: namespace.device,
        namespace_inode: namespace.inode,
        namespace_cookie: namespace.cookie,
        installation,
        binding: None,
        mark,
        table,
        start,
        pool_limit,
        pools: Vec::new(),
        rules: Vec::new(),
        routes: Vec::new(),
        tun: None,
        anchor: None,
        pending: None,
    })
}
