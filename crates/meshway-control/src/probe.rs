//! Local diagnostics of the Server connection; no credentials, no authority.
use crate::{Control, Error};

/// A read-only Server request over this Control's own protected client and
/// origin, for local diagnostics. It carries no credentials.
#[derive(Clone)]
pub struct ServerProbe {
    client: meshway_control_client::Client,
}

impl Control {
    pub fn server_probe(&self) -> ServerProbe {
        ServerProbe {
            client: self.client.clone(),
        }
    }
}

impl ServerProbe {
    /// Round trip of a read-only request after one warm-up request, so that
    /// connection setup is not part of the measurement.
    pub async fn round_trip(&self) -> Result<std::time::Duration, Error> {
        self.client.get_auth_options().await?;
        let started = std::time::Instant::now();
        self.client.get_auth_options().await?;
        Ok(started.elapsed())
    }
}
