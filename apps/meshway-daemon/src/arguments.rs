use clap::Parser;
#[derive(Parser)]
#[command(name = "meshwayd", version, about = "Meshway Node daemon")]
pub struct Arguments {
    /// Private machine state directory; its parent must already exist.
    #[arg(long, default_value = "/var/lib/meshway")]
    data_dir: std::path::PathBuf,
    /// Server origin. HTTPS is required outside numeric loopback.
    #[arg(long, env = "MESHWAY_SERVER_URL")]
    server: String,
    /// Browser console origin, when served separately from the Server.
    #[arg(long)]
    console_origin: Option<String>,
    /// Suggested label for first enrollment; later business settings live on Server.
    #[arg(long)]
    hostname: Option<String>,
}
impl Arguments {
    pub fn validate(self) -> Result<meshway_daemon::Settings, &'static str> {
        meshway_daemon::Settings::new(
            self.data_dir,
            &self.server,
            self.console_origin.as_deref(),
            self.hostname.as_deref(),
        )
    }
}
