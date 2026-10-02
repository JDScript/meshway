//! Narrow generated enrollment/scope operations over controller-only admission.
use super::ControllerChannel;
use meshway_agent_underlay::{BootstrapUnderlay, SystemDnsSnapshot};
use meshway_control_client::{
    client::{ApiOpError, EnrollAgentApiError, GetAgentDnsApiError, HeartbeatAgentApiError},
    types,
};
pub(crate) struct BootstrapChannel {
    channel: ControllerChannel,
}
impl BootstrapChannel {
    pub(crate) async fn prepare(
        owner: &BootstrapUnderlay,
        origin: reqwest::Url,
    ) -> std::io::Result<Self> {
        Ok(Self {
            channel: ControllerChannel::from_prepared(
                owner.controller_generation().await?,
                origin,
                SystemDnsSnapshot::from_system()?,
            )?,
        })
    }
    pub(crate) async fn enroll(
        &mut self,
        token: &str,
        request: types::EnrollAgentRequest,
    ) -> Result<types::AgentNodeResponse, Box<ApiOpError<EnrollAgentApiError>>> {
        self.channel
            .client()
            .with_api_key(token)
            .enroll_agent(request)
            .await
            .map_err(Box::new)
    }
    pub(crate) async fn heartbeat(
        &mut self,
        credential: &str,
        request: types::HeartbeatRequest,
    ) -> Result<types::AgentNodeResponse, Box<ApiOpError<HeartbeatAgentApiError>>> {
        self.channel
            .client()
            .with_api_key(credential)
            .heartbeat_agent(request)
            .await
            .map_err(Box::new)
    }
    pub(crate) async fn scope(
        &mut self,
        credential: &str,
    ) -> Result<types::AgentDnsSnapshot, Box<ApiOpError<GetAgentDnsApiError>>> {
        self.channel
            .client()
            .with_api_key(credential)
            .with_max_response_body_bytes(meshway_protocol::MAX_DNS_BYTES)
            .get_agent_dns()
            .await
            .map_err(Box::new)
    }
    pub(crate) async fn retire(self) {
        self.channel.retire_and_join().await;
    }
}
