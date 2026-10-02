//! Guard-only enrollment ownership. No business capability is exposed here.
use super::*;
pub struct BootstrapPreparation<'a> {
    pub state_directory: &'a Path,
    pub fingerprint: &'a str,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapWire {
    version: u8,
    fingerprint: String,
    namespace: Namespace,
    reservation: Reservation,
    installed: Vec<Installed>,
    pending: Option<Resource>,
    operation: Option<Operation>,
    cleaning: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(from = "BootstrapWire", into = "BootstrapWire")]
struct BootstrapJournal {
    version: u8,
    fingerprint: String,
    cleaning: bool,
    transactions: Transactions,
}

impl From<BootstrapWire> for BootstrapJournal {
    fn from(value: BootstrapWire) -> Self {
        Self {
            version: value.version,
            fingerprint: value.fingerprint,
            cleaning: value.cleaning,
            transactions: Transactions {
                namespace: value.namespace,
                reservation: value.reservation,
                installed: value.installed,
                pending: value.pending,
                operation: value.operation,
            },
        }
    }
}
impl From<BootstrapJournal> for BootstrapWire {
    fn from(value: BootstrapJournal) -> Self {
        Self {
            version: value.version,
            fingerprint: value.fingerprint,
            cleaning: value.cleaning,
            namespace: value.transactions.namespace,
            reservation: value.transactions.reservation,
            installed: value.transactions.installed,
            pending: value.transactions.pending,
            operation: value.transactions.operation,
        }
    }
}
impl std::ops::Deref for BootstrapJournal {
    type Target = Transactions;
    fn deref(&self) -> &Transactions {
        &self.transactions
    }
}
impl std::ops::DerefMut for BootstrapJournal {
    fn deref_mut(&mut self) -> &mut Transactions {
        &mut self.transactions
    }
}
impl OwnerJournal for BootstrapJournal {
    const FILE: &'static str = "bootstrap.json";
    fn restrict_resources(&self) -> io::Result<()> {
        if self.version != 1
            || self.fingerprint.len() != 64
            || self.namespace.boot.is_empty()
            || self.namespace.cookie.is_none_or(|cookie| cookie == 0)
            || !self
                .fingerprint
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.installed.len() > 4
            || self
                .installed
                .iter()
                .map(|i| &i.resource)
                .chain(self.pending.iter())
                .chain(self.operation.iter().flat_map(|op| &op.resources))
                .any(|r| !matches!(r, Resource::Guard { .. } | Resource::Nop { .. }))
            || self
                .operation
                .as_ref()
                .is_some_and(|op| op.start != 0 || op.phase == OperationPhase::Converge)
        {
            return Err(invalid(
                "invalid guard-only bootstrap journal; preserve resources",
            ));
        }
        Ok(())
    }
}

pub struct BootstrapUnderlay {
    core: NamespaceOwner<BootstrapJournal>,
}

impl BootstrapUnderlay {
    pub async fn prepare(input: BootstrapPreparation<'_>) -> io::Result<Self> {
        let lease = acquire_lease(input.state_directory)?;
        if input.state_directory.join("underlay.json").try_exists()? {
            return Err(invalid(
                "bound journal without verified binding; preserve resources",
            ));
        }
        let path = input.state_directory.join(BootstrapJournal::FILE);
        let previous = match private_file(&path, false) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(1_048_577).read_to_end(&mut bytes)?;
                if bytes.len() > 1_048_576 {
                    return Err(invalid("bootstrap journal exceeds bounds"));
                }
                Some(
                    serde_json::from_slice::<BootstrapJournal>(&bytes)
                        .map_err(|e| invalid(e.to_string()))?,
                )
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if let Some(journal) = previous {
            journal.restrict_resources()?;
            if journal.fingerprint != input.fingerprint {
                return Err(invalid(
                    "bootstrap enrollment fingerprint changed; no adoption",
                ));
            }
            if journal.namespace.same_lifetime(&lease.scope)? {
                if !journal.namespace.matching_locator(&lease.scope) {
                    return Err(invalid("bootstrap cookie locator mismatch"));
                }
                let mut core = NamespaceOwner {
                    lease,
                    directory: input.state_directory.into(),
                    journal,
                    #[cfg(test)]
                    checkpoint: None,
                };
                audit_unbound(&core.journal.installed).await?;
                core.validate_journal_resources()?;
                if core.journal.pending.is_some() {
                    return Err(invalid("ambiguous bootstrap mutation; preserve journal"));
                }
                if !core.journal.cleaning {
                    core.recover_operation().await?;
                    if core.journal.installed.len() != 4 {
                        core.rollback_partial_reservation().await?;
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "partial bootstrap rolled back; retry",
                        ));
                    }
                    core.verify().await?;
                }
                return Ok(Self { core });
            }
            audit_unbound(&[]).await?;
            // Archive a different lifetime only after auditing the fresh scope.
            let mut random = [0; 16];
            getrandom::fill(&mut random).map_err(|e| io::Error::other(e.to_string()))?;
            let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
            let archive = input
                .state_directory
                .join(format!("bootstrap-scope-{suffix}.json"));
            if archive.try_exists()? {
                return Err(invalid("bootstrap scope archive already exists"));
            }
            std::fs::rename(&path, &archive)?;
            File::open(input.state_directory)?.sync_all()?;
        }
        audit_unbound(&[]).await?;
        let mut rules = objects(4, true).await?;
        rules.extend(objects(6, true).await?);
        let mut routes = objects(4, false).await?;
        routes.extend(objects(6, false).await?);
        let reservation = select_reservation(
            &rules,
            &routes,
            &reserved_aliases()?,
            format!("enrollment/{}", input.fingerprint),
        )?;
        let journal = BootstrapJournal {
            version: 1,
            fingerprint: input.fingerprint.into(),
            cleaning: false,
            transactions: Transactions {
                namespace: lease.scope.clone(),
                reservation,
                installed: Vec::new(),
                pending: None,
                operation: None,
            },
        };
        write_journal(input.state_directory, &journal)?;
        let mut core = NamespaceOwner {
            lease,
            directory: input.state_directory.into(),
            journal,
            #[cfg(test)]
            checkpoint: crash_checkpoint("bootstrap"),
        };
        let first = core.journal.reservation.first_priority;
        let last = core.journal.reservation.last_priority;
        let mark = core.journal.reservation.mark.merge(0)?;
        core.install_transaction(vec![
            Resource::Nop {
                family: 4,
                priority: last,
            },
            Resource::Nop {
                family: 6,
                priority: last,
            },
            Resource::Guard {
                family: 4,
                priority: first,
                mark,
                target: last,
            },
            Resource::Guard {
                family: 6,
                priority: first,
                mark,
                target: last,
            },
        ])
        .await?;
        core.verify().await?;
        Ok(Self { core })
    }

    pub async fn controller_generation(&self) -> io::Result<PreparedGeneration> {
        if self.core.journal.cleaning {
            return Err(invalid("bootstrap is transitioning; no new HTTP"));
        }
        audit_unbound(&self.core.journal.installed).await?;
        self.core.prepare_generation(true).await
    }

    /// Caller must retire/join the narrow channel and durably save real binding
    /// first. The lease never leaves this move, including the singleton FD.
    pub async fn bind(mut self, input: Preparation<'_>) -> io::Result<LinuxUnderlay> {
        if input.state_directory != self.core.directory
            || input.identity.is_empty()
            || input.network.is_empty()
        {
            return Err(invalid("invalid bootstrap handoff binding"));
        }
        self.core.lease.check_current()?;
        if Arc::strong_count(&self.core.lease) != 1 {
            return Err(invalid(
                "bootstrap channel/socket drivers still retain namespace capability",
            ));
        }
        audit_unbound(&self.core.journal.installed).await?;
        self.core.journal.restrict_resources()?;
        if !self.core.journal.cleaning {
            self.core.verify().await?;
            self.core.journal.cleaning = true;
            write_journal(&self.core.directory, &self.core.journal)?;
        }
        self.core.rollback_partial_reservation().await?;
        LinuxUnderlay::prepare_with_lease(input, self.core.lease).await
    }
}

/// Shapes are rejection evidence only. No foreign link/route/rule is removed.
async fn audit_unbound(owned: &[Installed]) -> io::Result<()> {
    audit_marks().await?;
    let links: Vec<Value> = serde_json::from_slice(
        &ip(vec!["-d".into(), "-j".into(), "link".into(), "show".into()]).await?,
    )
    .map_err(|e| invalid(e.to_string()))?;
    for link in links {
        if !link.is_object() || link.get("ifindex").and_then(Value::as_u64).is_none() {
            return Err(invalid("cannot enumerate namespace links"));
        }
        if link
            .pointer("/linkinfo/info_kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| matches!(kind, "tun" | "tap" | "tuntap"))
        {
            return Err(invalid(
                "unexplained TUN/TAP before enrollment bootstrap; preserve link",
            ));
        }
    }
    for family in [4, 6] {
        for rule in objects(family, true).await? {
            if owned
                .iter()
                .any(|i| i.resource.family() == family && same_object(&rule, &i.observed))
            {
                continue;
            }
            if rule.get("goto").is_some()
                || rule.get("nop").is_some()
                || (rule.get("iif").and_then(Value::as_str) == Some("lo")
                    && rule
                        .get("table")
                        .map(table_identity)
                        .transpose()?
                        .is_some_and(|table| table > 255))
            {
                return Err(invalid(
                    "unexplained guard/nop before bootstrap; preserve rule",
                ));
            }
        }
        for route in objects(family, false).await? {
            if route
                .get("protocol")
                .and_then(|v| numeric_attribute("protocol", v))
                == Some(242)
            {
                return Err(invalid(
                    "legacy or unexplained Mesh route before bootstrap; preserve route",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
