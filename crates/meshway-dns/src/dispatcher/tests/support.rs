//! Private assembly only: enrollment, configuration and execution all use the
//! generated client and production Server/Control implementations.
use meshway_configuration::{EgressRule, NetworkConfigurationInput, RouteMatch, RuleAction};
use meshway_control::Control;
use meshway_protocol::{ConfigurationRequest, WireSecret};
use meshway_test_server::{
    TestServer,
    client::{ApplyConfigurationPath, GetConfigurationSourcePath, GetConfigurationSourceQuery},
    random_id as request_id,
};

const EMAIL: &str = "dispatcher-owner@example.test";
const PASSWORD: &str = "isolated dispatcher fixture password";

pub(super) struct Server(TestServer);

impl Server {
    pub(super) async fn start() -> Self {
        Self(TestServer::start().await)
    }

    pub(super) fn control(&self) -> Control {
        self.control_named("node")
    }

    pub(super) fn control_named(&self, name: &str) -> Control {
        self.0.open_control(name, meshway_test_server::http())
    }

    pub(super) async fn enroll_peer(&self, enrolled: &Control, peer: &mut Control) {
        let owner = self
            .0
            .client
            .login(&meshway_protocol::LoginRequest {
                email: EMAIL.into(),
                password: WireSecret::new(PASSWORD.into()),
            })
            .await
            .unwrap();
        self.0
            .enroll_into(
                &owner.token,
                enrolled.status().binding.unwrap().network_id,
                peer,
                "dispatcher-peer",
            )
            .await;
        peer.authenticate().await.unwrap();
    }

    pub(super) async fn enroll(&self, control: &mut Control) {
        let owner = self.0.owner(EMAIL, PASSWORD, "Dispatcher regression").await;
        let (session, network) = (&owner, owner.network);
        self.0.enroll(&owner, control, "dispatcher-node").await;

        let source = self
            .0
            .client
            .get_configuration_source(
                &GetConfigurationSourcePath {
                    network_id: network,
                },
                &GetConfigurationSourceQuery::default(),
                session.token.expose(),
            )
            .await
            .unwrap();
        let mut input = NetworkConfigurationInput::from(&source.configuration);
        input.dns.override_local = false;
        input.dns.servers.clear();
        input.dns.default.clear();
        input.dns.routes.clear();
        input.routing.egress.rules.push(EgressRule {
            remark: "Exact system resolver fixture destinations".into(),
            matcher: RouteMatch {
                ip_cidr: vec!["192.0.2.53/32".into(), "192.0.2.54/32".into()],
                ..Default::default()
            },
            action: RuleAction::Direct,
        });
        self.0
            .client
            .apply_configuration(
                &ApplyConfigurationPath {
                    network_id: network,
                },
                &ConfigurationRequest::Document {
                    expected_revision: source.revision,
                    request_id: request_id(),
                    source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
                },
                session.token.expose(),
            )
            .await
            .unwrap();
        control.authenticate().await.unwrap();
    }

    pub(super) async fn close(self) {
        self.0.close().await;
    }

    pub(super) fn real_control(&self) -> Control {
        self.0.proxy().unwrap().pass_through();
        self.control()
    }
}
