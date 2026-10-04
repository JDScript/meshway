use meshway_model::state::SystemState;
use std::path::PathBuf;

#[derive(Clone)]
pub struct Settings {
    pub(crate) data_dir: PathBuf,
    pub(crate) server: String,
    pub(crate) console_origin: String,
    pub(crate) hostname: String,
    pub(crate) system: SystemState,
}
impl Settings {
    pub fn new(
        data_dir: PathBuf,
        server: &str,
        console_origin: Option<&str>,
        hostname: Option<&str>,
    ) -> Result<Self, &'static str> {
        let server =
            meshway_control::server_origin(server).map_err(|_| "Invalid Server origin.")?;
        let console_origin = meshway_control::server_origin(console_origin.unwrap_or(&server))
            .map_err(|_| "Invalid Console origin.")?;
        let system = rustix::system::uname();
        let machine_hostname = system
            .nodename()
            .to_str()
            .map_err(|_| "System hostname is not UTF-8.")?
            .to_owned();
        let hostname = hostname
            .unwrap_or_else(|| machine_hostname.split('.').next().unwrap_or("meshway"))
            .to_ascii_lowercase();
        if !meshway_configuration::valid_node_hostname(&hostname) {
            return Err("Use --hostname with a single DNS label.");
        }
        Ok(Self {
            data_dir,
            server,
            console_origin,
            hostname,
            system: SystemState {
                hostname: machine_hostname,
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
                agent_version: env!("CARGO_PKG_VERSION").into(),
            },
        })
    }
}
