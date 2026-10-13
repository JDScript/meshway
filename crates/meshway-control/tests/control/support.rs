use meshway_model::RequestId;
use meshway_test_server::{Options, TestServer};
use std::time::Duration;

pub use meshway_test_server::Owner;

pub struct Server(TestServer);

impl std::ops::Deref for Server {
    type Target = TestServer;
    fn deref(&self) -> &TestServer {
        &self.0
    }
}

pub fn request() -> RequestId {
    meshway_test_server::random_id()
}
impl Server {
    pub async fn control(
        &self,
        owner: &Owner,
        origin: &str,
        name: &str,
        capabilities: std::collections::BTreeMap<String, meshway_model::state::Capability>,
    ) -> meshway_control::Control {
        let mut control =
            meshway_control::Control::open(&self.path(name), origin, meshway_test_server::http())
                .unwrap();
        self.0
            .connected(owner, &mut control, name, capabilities)
            .await;
        control
    }

    pub async fn start() -> Self {
        // An unchanged sync answers after one second instead of a full hold.
        Self(
            TestServer::start_with(Options {
                hold: Some(Duration::from_secs(1)),
                pin: false,
            })
            .await,
        )
    }
    pub async fn owner(&self) -> Owner {
        self.0
            .owner(
                "node-control@example.test",
                "real control fixture password",
                "Control fixture",
            )
            .await
    }
    pub async fn close(self) {
        self.0.close().await;
    }
}

/// Peer facts are irrelevant to these Control fixtures.
use meshway_test_server::Facts as NoFacts;

/// One real `sync` exchange; returns the netmap Control now holds.
pub async fn sync(
    control: &mut meshway_control::Control,
) -> Result<Option<meshway_model::NetmapRef>, meshway_control::Error> {
    control
        .sync(&mut NoFacts::default())
        .await
        .map(|outcome| Some(outcome.netmap.content.reference()))
}

/// Save receipt of the held netmap for the current instance.
pub async fn acknowledge_saved(
    control: &mut meshway_control::Control,
) -> Result<meshway_control::ApplicationTicket, meshway_control::Error> {
    let netmap = control
        .netmap_reference()
        .ok_or(meshway_control::Error::Lifecycle)?;
    control.begin_application(netmap).await
}

/// An enrolled, authenticated Control with a Native UDP capable instance.
pub async fn udp_control(
    server: &Server,
    owner: &Owner,
    origin: &str,
    name: &str,
) -> meshway_control::Control {
    server
        .control(
            owner,
            origin,
            name,
            std::collections::BTreeMap::from([meshway_test_server::configuration::native_udp()]),
        )
        .await
}
