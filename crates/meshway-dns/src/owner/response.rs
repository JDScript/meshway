//! Output checks of the query owner's answers.
use super::*;

impl Answer {
    pub(crate) fn message(&self) -> &hickory_proto::op::Message {
        &self.ready.message
    }
    pub(crate) fn message_mut(&mut self) -> &mut hickory_proto::op::Message {
        &mut self.ready.message
    }
    pub(crate) fn query(&self) -> &Query {
        &self.ready.query
    }
    /// The query's deadline: an answer must be sent before it.
    pub(crate) fn deadline(&self) -> Instant {
        self.ready.deadline
    }
    pub(crate) fn answers(&self, query: &Query) -> bool {
        self.ready.query == *query
    }
    pub fn ensure_current(&self) -> Result<(), OwnerError> {
        if self.ready.cancelled.is_cancelled() || Instant::now() >= self.ready.deadline {
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
    pub async fn ended(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        tokio::select! { biased;
            _ = self.ready.cancelled.cancelled() => {},
            _ = tokio::time::sleep_until(self.ready.deadline.into()) => {},
            _ = async { match &self.ready.route { Some(route) => route.ended().await, None => std::future::pending().await } } => {},
        }
    }
}
