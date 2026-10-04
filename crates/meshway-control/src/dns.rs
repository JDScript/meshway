use crate::{Control, Error};
use meshway_configuration::normalize_dns_name;
use meshway_protocol::node::{NodeDnsQueryRequest, NodeDnsQueryResponse};

impl Control {
    /// Read an exact-name discovery snapshot with the current Node session.
    /// The caller owns DNS serving/caching; this read creates no execution lease.
    pub async fn query_dns(&mut self, name: &str) -> Result<NodeDnsQueryResponse, Error> {
        self.token()?;
        let request = NodeDnsQueryRequest {
            name: normalize_dns_name(name).map_err(|_| Error::InvalidDnsName)?,
        };
        let response = self.client.query_node_dns(&request, self.token()?).await;
        let response = self.registered(response)?;
        // Transit time cannot keep an expired session alive. This uses the same
        // monotonic/high-water clock as every other operation and snapshot.
        self.token()?;
        if self.record.binding.as_ref().is_none_or(|binding| {
            response.network_id != binding.network_id || response.source_node_id != binding.node_id
        }) || response
            .record
            .as_ref()
            .is_some_and(|record| record.name != request.name)
        {
            self.fence();
            return Err(Error::Contract);
        }
        Ok(response)
    }
}
