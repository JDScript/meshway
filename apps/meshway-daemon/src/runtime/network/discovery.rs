//! DNS discovery enters the same bounded request queue as packet demand work.
//! Dropping the query future closes its reply receiver, cancelling queued or
//! in-flight HTTP without another Control instance or a copied credential.
use super::Request;
use meshway_dns::{DiscoveryGateway, DnsFuture, GatewayError};
use meshway_protocol::NodeDnsQueryResponse;
use tokio::sync::{mpsc, oneshot};

pub(super) struct Gateway {
    sender: mpsc::Sender<Request>,
}

impl Gateway {
    pub fn new(sender: mpsc::Sender<Request>) -> Self {
        Self { sender }
    }
}

impl DiscoveryGateway for Gateway {
    fn query<'a>(
        &'a self,
        name: &'a str,
    ) -> DnsFuture<'a, Result<NodeDnsQueryResponse, GatewayError>> {
        Box::pin(async move {
            if name.len() > 253
                || meshway_configuration::normalize_dns_name(name)
                    .ok()
                    .as_deref()
                    != Some(name)
            {
                return Err(GatewayError);
            }
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            let (reply, received) = oneshot::channel();
            self.sender
                .try_send(Request::Dns {
                    name: name.to_owned(),
                    deadline,
                    reply,
                })
                .map_err(|_| GatewayError)?;
            tokio::time::timeout_at(deadline, received)
                .await
                .map_err(|_| GatewayError)?
                .map_err(|_| GatewayError)?
        })
    }
}
