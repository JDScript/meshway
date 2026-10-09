//! Output fences of the query owner's answers.
use super::*;

impl Answer {
    pub(crate) fn answers(&self, query: &Query) -> bool {
        self.ready.query == *query
    }
    pub(crate) fn with_scope_fence(mut self, fence: crate::dispatcher::ScopeFence) -> Self {
        self.scope_fence = Some(Box::new(fence));
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
