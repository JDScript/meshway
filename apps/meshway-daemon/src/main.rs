#![forbid(unsafe_code)]

// musl's allocator retains hundreds of MiB under the bounded packet/future
// allocation mix. Select the measured allocator at the executable boundary;
// libraries continue to use their caller's allocator.
#[cfg(all(target_os = "linux", target_env = "musl"))]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(unix)]
mod arguments;

#[cfg(unix)]
#[tokio::main]
async fn main() -> std::process::ExitCode {
    use clap::Parser;
    let arguments = arguments::Arguments::parse();
    if let Err(error) = arguments.logging.init() {
        eprintln!("{error}");
        return std::process::ExitCode::FAILURE;
    }
    let result = match arguments.validate() {
        Ok(settings) => run(settings).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error, "Node stopped with an error");
            std::process::ExitCode::FAILURE
        }
    }
}
#[cfg(unix)]
async fn run(settings: meshway_daemon::Settings) -> Result<(), &'static str> {
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .map_err(|_| "Cannot install signal handler.")?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "Cannot install signal handler.")?;
    meshway_daemon::run(settings, async {
        let signal = tokio::select! {
            _ = interrupt.recv() => "SIGINT",
            _ = terminate.recv() => "SIGTERM",
        };
        tracing::info!(signal, "Node shutdown requested");
    })
    .await
}
#[cfg(not(unix))]
fn main() {
    eprintln!("This Node build requires Unix local sockets.");
    std::process::exit(1);
}
