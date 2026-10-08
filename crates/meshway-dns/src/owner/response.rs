//! Output fences and exact discovery-response binding for the query owner.
use super::*;

impl Answer {
    pub(crate) fn answers(&self, query: &Query) -> bool {
        self.ready.query == *query
    }
    pub(crate) fn with_scope_fence(mut self, fence: crate::dispatcher::ScopeFence) -> Self {
        self.scope_fence = Some(fence);
        self
    }
    pub fn ensure_current(&self) -> Result<(), OwnerError> {
        self.snapshot
            .ensure_current()
            .map_err(|_| OwnerError::Inactive)?;
        if self.ready.cancelled.is_cancelled()
            || Instant::now() >= self.ready.deadline
            || self
                .scope_fence
                .as_ref()
                .is_some_and(|fence| !fence.current())
        {
            return Err(OwnerError::Cancelled);
        }
        if let Some(route) = &self.ready.route {
            route.ensure_current().map_err(|_| OwnerError::Cancelled)?;
        }
        Ok(())
    }
    pub fn encode(
        &self,
        transport: ResponseTransport,
        udp_payload_limit: u16,
    ) -> Result<Bytes, OwnerError> {
        self.ensure_current()?;
        let bytes = encode_response(
            &self.ready.query,
            &self.ready.message,
            transport,
            udp_payload_limit,
        )?;
        self.ensure_current()?;
        Ok(bytes)
    }
    pub async fn invalidated(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        tokio::select! { biased;
            _ = self.snapshot.invalidated() => {},
            _ = self.ready.cancelled.cancelled() => {},
            _ = tokio::time::sleep_until(self.ready.deadline.into()) => {},
            _ = async { match &self.ready.route { Some(route) => route.invalidated().await, None => std::future::pending().await } } => {},
            _ = async { match &self.scope_fence { Some(fence) => fence.invalidated().await, None => std::future::pending().await } } => {},
        }
    }
}

impl Binding {
    pub(super) fn validate(
        &self,
        pending: &Pending,
        response: &NodeDnsQueryResponse,
    ) -> Result<Option<RData>, Unavailable> {
        if response.network_id != self.network || response.source_node_id != self.source {
            return Err(Unavailable::BindingMismatch);
        }
        if response.configuration_revision != self.revision {
            return Err(Unavailable::ReloadRequired);
        }
        let record = response.record.as_ref().ok_or(Unavailable::NotVisible)?;
        if record.name != pending.name
            || normalize_dns_name(&record.name).ok().as_ref() != Some(&record.name)
        {
            return Err(Unavailable::BindingMismatch);
        }
        if (record.node_id == self.source) != (record.name == self.self_name)
            || (record.node_id == self.source && record.addresses != self.self_addresses)
            || self
                .peers
                .get(&record.node_id)
                .is_some_and(|addresses| *addresses != record.addresses)
        {
            return Err(Unavailable::BindingMismatch);
        }
        // Partial peers cannot prove absence or identify a target by its address.
        // The Server's exact-name response binds previously undiscovered nodes.
        match pending.query.question().query_type() {
            RecordType::A => Ok(record
                .addresses
                .ipv4
                .map(|address| RData::A(rdata::A(address)))),
            RecordType::AAAA => Ok(record
                .addresses
                .ipv6
                .map(|address| RData::AAAA(rdata::AAAA(address)))),
            _ => Err(Unavailable::UnsupportedRecordType),
        }
    }
}

pub(super) fn discovery_name(query: &Query) -> Option<String> {
    // DNS wire labels are arbitrary octets. Escaped presentation or lossy UTF-8
    // cannot become HTTP names; a label dot must not become a separator.
    let mut labels = Vec::new();
    for label in query.question().name().iter() {
        let label = std::str::from_utf8(label).ok()?;
        if label
            .chars()
            .any(|ch| matches!(ch, '.' | '\u{3002}' | '\u{ff0e}' | '\u{ff61}'))
        {
            return None;
        }
        labels.push(label);
    }
    normalize_dns_name(&labels.join(".")).ok()
}
