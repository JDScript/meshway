//! Installing the exit role: the owner task's event loop (audit, confirmation
//! polls, maintenance, requests) and its cleanup retries; reconciling a
//! stopped role and releasing it for a Host transition.
use super::*;

/// Covers cancellation of the Host installation future after its first owned
/// resource is allocated. Restart reconciles the durable intent before reuse.
struct Installing(Option<Arc<crate::linux::protection::Shared>>);
impl Drop for Installing {
    fn drop(&mut self) {
        if let Some(shared) = self.0.take() {
            // A cancelled installation proves no other writer: the Host
            // re-proves what the journal and the kernel hold.
            shared.end(crate::OwnershipEnd::Unavailable {
                check: "install_cancelled",
            });
        }
    }
}

/// The audit proved the role's objects changed, or could not read them for
/// the whole limit: the owner stops. A foreign change is kept with the exit
/// role's ownership (it does not install again until a re-read finds that
/// evidence gone); an unreadable or removed one is the role's own failure,
/// retried by its reinstallation.
fn audit_ended(check: &'static str, verification: &Verification, proven: &Mutex<Option<String>>) {
    tracing::warn!(operation = "audit", check, error = %verification, "Exit owner stopping after a failed kernel operation");
    // An owned object read back changed, or a flow no projection owns, is
    // someone else's object. One that is only missing (removed outside the
    // owner) leaves nothing foreign once the role's cleanup is done: the
    // role installs again then.
    if let Verification::Changed(what) = verification
        && !what.ends_with("_missing")
        && let Ok(mut proven) = proven.lock()
    {
        proven.get_or_insert_with(|| verification.to_string());
    }
}

/// A failed projection change stops the whole owner; say why.
fn stopped(operation: &'static str, check: &'static str, error: &Error) {
    tracing::warn!(operation, check, %error, "Exit owner stopping after a failed kernel operation");
}

/// What a retried change needs from the owner: where its pending retry is
/// published, and what ends its wait.
struct Retrying<'a> {
    changes: &'a tokio::sync::watch::Sender<Option<crate::KernelChangePending>>,
    stops: [&'a CancellationToken; 3],
    /// What proved an exit-owned object changed, if the owner stopped for it.
    proven: &'a Mutex<Option<String>>,
}

impl Retrying<'_> {
    /// One kernel change of the owner. A failed attempt is read back and
    /// retried while the owned objects read back consistent with the
    /// journal or cannot be read, at most 30 s (see [`retried`]); then, or
    /// at once on a read-back that differs, the owner stops as before.
    async fn change(
        &self,
        backend: &mut Backend,
        operation: &'static str,
        run: impl for<'a> FnMut(&'a mut Backend) -> Boxed<'a, Result<()>>,
        reread: impl for<'a> FnMut(
            &'a mut Backend,
        ) -> Boxed<'a, super::super::super::verification::Reread>,
    ) -> Result<()> {
        retried(
            backend,
            Verified::Exit,
            operation,
            self.changes,
            &self.stops,
            run,
            reread,
        )
        .await
        .map_err(|(error, check)| {
            if check != "stopped" {
                stopped(operation, check, &error);
            }
            let proven = check == "verify"
                || (check == "refused"
                    && super::super::super::kind(&error) == super::super::super::Kind::Proven);
            if proven && let Ok(mut recorded) = self.proven.lock() {
                recorded.get_or_insert_with(|| format!("{operation}: {error}"));
            }
            error
        })
    }

    /// Retire one projection whose operation failed or was answered to
    /// nobody (each step reads back and skips what is already done).
    async fn reconcile(
        &self,
        backend: &mut Backend,
        operation: &'static str,
        incarnation: super::super::ExitIncarnation,
    ) -> Result<()> {
        self.change(
            backend,
            operation,
            move |backend: &mut Backend| Box::pin(backend.failed(incarnation)),
            move |backend: &mut Backend| Box::pin(backend.reread(incarnation)),
        )
        .await
    }
}

impl super::super::super::Kernel {
    /// Keep uncertain cleanup attached to its owner. A stopped request queue is
    /// not an ownership conflict, and a failed cleanup is not an absence proof:
    /// while it is retried this is `Conflict`, after it completed the role
    /// may install again.
    pub(in crate::linux::kernel::exit) async fn reconcile_stopped_exit(&mut self) -> Result<()> {
        if let Some(owner) = self.exit.as_mut()
            && !owner.role.is_active()
        {
            owner.shutdown().await?;
            self.quiesce().await?;
            let mut expected = self.expected.lock().map_err(|_| Error::Worker)?;
            expected.exit_tun = None;
            expected.exit_tables.clear();
            expected.exit_stop = None;
            self.exit.take();
        }
        Ok(())
    }

    /// End the exit role for a Host transition (new content, suspension). A
    /// role that already stopped on its own failure was reported then; its
    /// kept resources are inert (closed gate, no TUN) and stay recorded in
    /// its journal and the monitor's expectations, so they never block the
    /// Host's own resources. Retiring a live role must succeed.
    pub(in crate::linux::kernel) async fn release_exit(&mut self) -> Result<()> {
        let Some(mut owner) = self.exit.take() else {
            return Ok(());
        };
        let stopped = !owner.role.is_active();
        let result = owner.shutdown().await;
        if result.is_err() && owner.role.cleanup_pending().is_some() {
            // Its owner retries the cleanup; its objects stay expected.
            tracing::warn!("Exit role cleanup pending; its owner retries it");
            self.exit = Some(owner);
            return if stopped { Ok(()) } else { result };
        }
        self.expected.lock().map_err(|_| Error::Worker)?.exit_stop = None;
        match result {
            Ok(()) => Ok(()),
            Err(error) if stopped => {
                tracing::warn!(%error, "Failed exit role keeps its resources until recovery");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Whether a proven change of an exit-owned object still keeps the role
    /// withdrawn. Read again at most every 60 s, read only: the evidence is
    /// gone once the stopped role's cleanup removed everything it owned.
    pub(in crate::linux::kernel) async fn exit_blocked(&mut self) -> bool {
        let meshway_model::recovery::Health::Blocked(blocked) = self.shared.exit.health() else {
            return false;
        };
        let now = Instant::now();
        if blocked.reread_at.is_some_and(|at| now < at) {
            return true;
        }
        let cleaned = self.reconcile_stopped_exit().await.is_ok() && self.exit.is_none();
        let observation = if cleaned {
            meshway_model::recovery::Observation::Holds
        } else {
            meshway_model::recovery::Observation::proven(blocked.check, blocked.reason.clone())
        };
        self.shared.exit.observe(observation, now);
        if cleaned {
            tracing::info!(
                check = blocked.check,
                blocked_ms = now.duration_since(blocked.since).as_millis() as u64,
                "Exit role ownership read again: its changed objects are gone; the role may install again"
            );
        }
        !cleaned
    }

    pub async fn install_exit_role(&mut self, plan: RoleIdentity) -> Result<ExitRole> {
        self.ready().await?;
        if self.exit_blocked().await {
            return Err(Error::Conflict);
        }
        self.reconcile_stopped_exit().await?;
        if let Some(owner) = &self.exit {
            if owner.role.plan == plan && owner.role.is_active() {
                return Ok(owner.role.clone());
            }
            return Err(Error::Conflict);
        }
        let desired = self.shared.installed()?.ok_or(Error::Expired)?;
        let cfg = View::of(&desired);
        if cfg.exit_role().as_ref() != Some(&plan)
            || !cfg.exit_enabled()
            || !plan.families.contains(&ExitFamily::Ipv4)
        {
            return Err(Error::Target);
        }
        if std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")?.trim() != "1" {
            return Err(Error::Target);
        }
        self.verify().await?;
        let network = cfg.network_id();
        let node = cfg.node_id();
        let pools: Vec<_> = cfg
            .mesh_address_pools()
            .ipv4
            .iter()
            .map(|pool| pool.parse::<ipnet::Ipv4Net>().map_err(|_| Error::Target))
            .collect::<Result<_>>()?;
        if pools.is_empty() || self.journal.start <= super::super::routing::RESERVED {
            return Err(Error::Target);
        }
        let ipv6 = plan.families.contains(&ExitFamily::Ipv6);
        let pools6: Vec<_> = if ipv6 {
            cfg.mesh_address_pools()
                .ipv6
                .iter()
                .map(|pool| pool.parse::<ipnet::Ipv6Net>().map_err(|_| Error::Target))
                .collect::<Result<_>>()?
        } else {
            Vec::new()
        };
        let rules =
            super::super::super::get_rules(&self.handle, self.options.max_kernel_objects).await?;
        let routes =
            super::super::super::get_routes(&self.handle, self.options.max_kernel_objects).await?;
        let mask = self
            .shared
            .selectors
            .iter()
            .fold(self.journal.mark.mask, |mask, selector| {
                mask | selector.mask
            });
        let (store, prior) = Store::open(self.store.directory()).await?;
        if prior.as_ref().is_some_and(|ledger| !ledger.removed) {
            return Err(Error::Conflict);
        }
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| Error::Worker)?;
        let installation: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut commands = Commands::default();
        let mut owned = Vec::new();
        if let Some(egress) = &self.egress {
            owned.extend(egress.capture.owned_tables().await?);
        }
        super::super::prerequisites::check(&mut commands, &owned).await?;
        let mut selected = None;
        for shift in 0..32 {
            let bit = 1u32 << ((shift + u32::from(random[0])) % 32);
            if mask & bit == 0
                && super::super::conntrack::mark_unused(bit)
                    .await
                    .map_err(|_| Error::Conflict)?
            {
                selected = Some(bit);
                break;
            }
        }
        let mark = selected.ok_or(Error::Conflict)?;
        let seed = u32::from_be_bytes(random[..4].try_into().map_err(|_| Error::Worker)?);
        let table = (0..1024)
            .map(|offset| 10000 + seed.wrapping_add(offset) % (u32::MAX - 20000))
            .find(|table| {
                *table != self.journal.table
                    && !routes
                        .iter()
                        .any(|route| super::super::super::plan::route_table(route) == *table)
                    && !rules
                        .iter()
                        .any(|rule| super::super::super::plan::rule_table(rule) == *table)
            })
            .ok_or(Error::Conflict)?;
        let local: Vec<IpAddr> =
            super::super::super::get_addresses(&self.handle, self.options.max_kernel_objects)
                .await?
                .into_iter()
                .flat_map(|address| address.attributes)
                .filter_map(|attribute| match attribute {
                    rtnetlink::packet_route::address::AddressAttribute::Address(ip)
                    | rtnetlink::packet_route::address::AddressAttribute::Local(ip) => Some(ip),
                    _ => None,
                })
                .collect();
        let mut installing = Installing(Some(self.shared.clone()));
        let tun = {
            let mut expected = self.expected.lock().map_err(|_| Error::Worker)?;
            // Offload: reverse-NAT replies of a bulk flow reach the reader as
            // one aggregate per kernel queue slot instead of one per segment.
            let tun = meshway_tun::Tun::create(
                meshway_tun::Options::new(self.options.mtu)?.with_batch_io(true),
            )?;
            expected.exit_tun = Some(tun.if_index()?);
            expected.exit_tables.push(table);
            tun
        };
        self.quiesce().await?;
        let namespace = super::super::super::namespace::Identity::current()?;
        let ledger = Ledger {
            version: 2,
            boot: namespace.boot,
            namespace_device: namespace.device,
            namespace_inode: namespace.inode,
            namespace_cookie: namespace.cookie,
            base_installation: self.journal.installation.clone(),
            network,
            node,
            plan: plan.clone(),
            program: Program {
                legacy: false,
                installation,
                interface: tun.if_index()?,
                interface_name: tun.name().to_owned(),
                mark,
                pools,
                families: if ipv6 {
                    vec![ExitFamily::Ipv4, ExitFamily::Ipv6]
                } else {
                    vec![ExitFamily::Ipv4]
                },
                pools6,
            },
            tun_name: tun.name().to_owned(),
            mtu: tun.mtu(),
            route_table: table,
            rules: Vec::new(),
            routes: Vec::new(),
            receipt: None,
            projections: Vec::new(),
            quarantine: prior
                .as_ref()
                .map_or_else(Vec::new, |ledger| ledger.quarantine.clone()),
            legacy_quarantine: prior.map_or_else(Vec::new, |ledger| ledger.legacy_quarantine),
            closing: false,
            removed: false,
            pending: true,
        };
        let view = Arc::new(RwLock::new(BTreeMap::new()));
        let stop = CancellationToken::new();
        self.expected.lock().map_err(|_| Error::Worker)?.exit_stop = Some(stop.clone());
        let routing = Routing {
            handle: self.handle.clone(),
            expected: self.expected.clone(),
            limit: self.options.max_kernel_objects,
            start: self.journal.start,
            end: self.journal.end(),
        };
        let mut backend =
            Backend::create(store, ledger, routing, view.clone(), local, stop.clone()).await?;
        self.quiesce().await?;
        let authority = Authority::new(self.shared.clone(), plan.clone());
        let (mut io, writer, reader) =
            io::Owner::start(tun, view.clone(), stop.clone(), authority.clone());
        let (sender, mut receiver) = mpsc::channel(64);
        let (done, status) = tokio::sync::watch::channel(Closing::Serving);
        let role = ExitRole {
            sender,
            writer,
            reader: Arc::new(Mutex::new(Some(reader))),
            stop: stop.clone(),
            done: status,
            view,
            authority: authority.clone(),
            plan,
        };
        let conflict = self.shared.conflict.clone();
        let interface_name = backend.ledger.tun_name.clone();
        let shutdown = self.shared.tasks.stop.clone();
        let shared = self.shared.clone();
        let task = tokio::spawn(async move {
            let proven = Mutex::new(None::<String>);
            let retrying = Retrying {
                changes: &shared.changes,
                stops: [&stop, &conflict, &shutdown],
                proven: &proven,
            };
            // Confirmations wait for a kernel fact; they never hold the owner.
            let mut confirming: Vec<(
                super::super::ExitIncarnation,
                Instant,
                oneshot::Sender<Result<CtConfirmation>>,
            )> = Vec::new();
            let mut next_poll = Instant::now();
            // Each duty is due a full period after its last run ended, so a
            // slow one never starves the requests queued behind it.
            let mut tick = Cadence::new(Duration::from_millis(100));
            let mut audit = Cadence::new(Duration::from_secs(1));
            let mut unverified = Unverified::new(Verified::Exit);
            loop {
                tokio::select! { biased;
                    _ = stop.cancelled() => break,
                    _ = conflict.cancelled() => break,
                    _ = shutdown.cancelled() => break,
                    // A read that fails is unknown, retried next second: the
                    // role and its flows stay. Read back and different, or
                    // unknown for 30 s, ends it.
                    _ = audit.due() => {
                        let verification = backend.check().await;
                        audit.ran();
                        if let Err(reason) = unverified.record(&verification, Instant::now()) { audit_ended(reason, &verification, &proven); break; }
                    },
                    _ = tokio::time::sleep_until(next_poll.into()), if !confirming.is_empty() => {
                        let mut failed = false;
                        let mut waiting = Vec::with_capacity(confirming.len());
                        for (incarnation, until, reply) in std::mem::take(&mut confirming) {
                            let result = match backend.poll_confirm(incarnation, until).await {
                                Ok(None) => {
                                    waiting.push((incarnation, until, reply));
                                    continue;
                                }
                                Ok(Some(confirmation)) => Ok(confirmation),
                                Err(error) => {
                                    tracing::debug!(operation = "confirm", %error, "Exit projection operation failed");
                                    Err(error)
                                }
                            };
                            let error = result.is_err();
                            let sent = reply.send(result).is_ok();
                            if (error || !sent) && retrying.reconcile(&mut backend, "confirm", incarnation).await.is_err() {
                                failed = true;
                                break;
                            }
                        }
                        confirming.extend(waiting);
                        next_poll = Instant::now() + super::super::backend::CONFIRM_POLL;
                        if failed { break; }
                    },
                    _ = tick.due() => {
                        // Withdrawal, Host suspension or a withdrawn role stops the
                        // owner: every projection retires with its exact CT.
                        if !authority.keeps_role() { tracing::debug!(operation = "role", "Exit owner stopping: role no longer installed"); break; }
                        // Projection cutoffs and explicit revocations.
                        if retrying.change(&mut backend, "maintain", |backend: &mut Backend| Box::pin(backend.maintain()), |backend: &mut Backend| Box::pin(backend.reread_maintain())).await.is_err() { break; }
                        tick.ran();
                    },
                    request = receiver.recv() => {
                        let Some(request) = request else { break; };
                        match request {
                            Request::Audit { reply } => {
                                let verification = backend.check().await;
                                let ended = unverified.record(&verification, Instant::now());
                                let _ = reply.send(match &verification {
                                    Verification::Holds => Ok(backend.ledger.program.tables().into_iter().map(|(family, name)| (family.to_owned(), name)).collect()),
                                    Verification::Unavailable(_) => Err(Error::Netlink),
                                    Verification::Changed(_) => Err(Error::Conflict),
                                });
                                if let Err(reason) = ended { audit_ended(reason, &verification, &proven); break; }
                            },
                            Request::Install { items, reply } => {
                                // Rejected queued work never entered the kernel
                                // transaction and must not retire a kept flow.
                                let mut results: Vec<Option<Result<ProjectionAck>>> = Vec::new();
                                let mut admitted = Vec::new();
                                let mut guards = Vec::new();
                                for (item, guard) in items {
                                    if guard.check().is_err() {
                                        results.push(Some(Err(Error::Expired)));
                                    } else {
                                        results.push(None);
                                        admitted.push(item);
                                        guards.push(guard);
                                    }
                                }
                                let incarnations: Vec<_> = admitted.iter().map(|item| item.incarnation).collect();
                                let installed = backend.install_batch(admitted).await;
                                let mut failed = false;
                                let mut installed = installed.into_iter();
                                for (incarnation, guard) in incarnations.iter().zip(&guards) {
                                    let result = installed.next().unwrap_or(Err(Error::Worker));
                                    if let Err(error) = &result { tracing::debug!(operation = "install", step = backend.take_failed_step(*incarnation), %error, "Exit projection operation failed"); }
                                    let result = result.and_then(|ack| { guard.check()?; Ok(ack) });
                                    if result.is_err() && retrying.reconcile(&mut backend, "install", *incarnation).await.is_err() { failed = true; break; }
                                    if let Some(slot) = results.iter_mut().find(|slot| slot.is_none()) { *slot = Some(result); }
                                }
                                if failed { break; }
                                let results: Vec<_> = results.into_iter().map(|result| result.unwrap_or(Err(Error::Worker))).collect();
                                let delivered: Vec<_> = results.iter().map(|result| result.as_ref().ok().map(|ack| ack.incarnation)).collect();
                                if reply.send(results).is_err() {
                                    // Nobody takes these acknowledgements: no flow may use them.
                                    for incarnation in delivered.into_iter().flatten() {
                                        if retrying.reconcile(&mut backend, "install", incarnation).await.is_err() { failed = true; break; }
                                    }
                                    if failed { break; }
                                }
                            },
                            Request::Renew(renew) => {
                                let Renew { incarnation, deadline, scope, guard, reply } = *renew;
                                if guard.check().is_err() {
                                    let _ = reply.send(Err(Error::Expired));
                                    continue;
                                }
                                let result = backend.renew(incarnation, deadline, scope).await;
                                if let Err(error) = &result { tracing::debug!(operation = "renew", %error, "Exit projection operation failed"); }
                                let result = result.and_then(|ack| { guard.check()?; Ok(ack) });
                                if result.is_err() && retrying.reconcile(&mut backend, "renew", incarnation).await.is_err() { break; }
                                if reply.send(result).is_err() && retrying.reconcile(&mut backend, "renew", incarnation).await.is_err() { break; }
                            },
                            Request::Confirm { incarnation, reply } => match backend.confirmation_cutoff(incarnation) {
                                Ok(until) => {
                                    if confirming.is_empty() {
                                        next_poll = Instant::now() + super::super::backend::CONFIRM_POLL;
                                    }
                                    confirming.push((incarnation, until, reply));
                                }
                                Err(error) => {
                                    tracing::debug!(operation = "confirm", %error, "Exit projection operation failed");
                                    let _ = reply.send(Err(error));
                                    if retrying.reconcile(&mut backend, "confirm", incarnation).await.is_err() { break; }
                                }
                            },
                            Request::Retire { incarnation, reply } => {
                                let result = retrying.reconcile(&mut backend, "retire", incarnation).await;
                                let failed = result.is_err();
                                let _ = reply.send(result);
                                if failed { break; }
                            },
                        }
                    },
                }
            }
            stop.cancel();
            if let Some(reason) = proven.lock().ok().and_then(|mut proven| proven.take()) {
                shared.exit.observe(
                    meshway_model::recovery::Observation::proven(
                        "exit_verify",
                        format!("exitOwnership: {reason}"),
                    ),
                    Instant::now(),
                );
            }
            // A stopped owner serves nothing more: requests queued before it
            // stopped are dropped now (their callers see `Worker` at once),
            // never left waiting on its cleanup retries below.
            receiver.close();
            while receiver.try_recv().is_ok() {}
            drop(std::mem::take(&mut confirming));
            let first = backend.begin_close(true).await;
            let io_result = io.shutdown().await;
            let mut result = match (first, io_result) {
                (Ok(()), Ok(())) => backend.finish_close().await,
                (Err(error), _) | (_, Err(error)) => Err(error),
            };
            // A failed cleanup keeps its resources and is retried from what
            // the kernel holds then, until it completes or the Host ends.
            // Its own recovery: retried without a total limit, until it
            // completes or the Host's tasks stop.
            let mut cleanup =
                meshway_model::recovery::Recovery::new(meshway_model::recovery::Policy {
                    backoff: meshway_model::Backoff::new(CLEANUP_RETRY, CLEANUP_RETRY_MAX),
                    horizon: meshway_model::recovery::Horizon::Unbounded,
                    reread_blocked: None,
                });
            while let Err(error) = result {
                let now = Instant::now();
                let at = match cleanup.observe(
                    meshway_model::recovery::Observation::unknown("exit_cleanup", &error),
                    now,
                ) {
                    meshway_model::recovery::Step::RetryAt(at) => at,
                    _ => now + CLEANUP_RETRY_MAX,
                };
                let (attempts, wait) = (cleanup.attempts(), at.saturating_duration_since(now));
                if attempts.is_power_of_two() {
                    tracing::warn!(attempts, %error, retry_ms = wait.as_millis() as u64, "Exit role cleanup failed; resources kept, retrying");
                }
                let pending = cleanup.health().pending().unwrap_or_else(|| RetryPending {
                    reason: error.to_string(),
                    attempts,
                    retry_at: std::time::SystemTime::now() + wait,
                });
                shared.cleanups.send_replace(Some(pending.clone()));
                done.send_replace(Closing::Pending(pending));
                tokio::select! { biased;
                    _ = shutdown.cancelled() => {
                        done.send_replace(Closing::Done(false));
                        shared.cleanups.send_replace(None);
                        return;
                    },
                    _ = tokio::time::sleep(wait) => {},
                }
                result = backend.close_observed(true).await;
            }
            let attempts = cleanup.attempts() + 1;
            if attempts > 1 {
                tracing::info!(
                    attempts,
                    "Exit role cleanup completed after retrying; the role may install again"
                );
            }
            done.send_replace(Closing::Done(true));
            shared
                .cleanups
                .send_if_modified(|pending| pending.take().is_some());
        });
        self.exit = Some(Owner {
            role: role.clone(),
            interface_name,
            task: Some(task),
        });
        installing.0 = None;
        Ok(role)
    }
}
