#![forbid(unsafe_code)]
#[cfg(unix)]
mod arguments;
#[cfg(unix)]
mod commands;
#[cfg(unix)]
mod output;

#[cfg(unix)]
#[tokio::main]
async fn main() -> std::process::ExitCode {
    use arguments::Arguments;
    use clap::Parser;
    let arguments = Arguments::parse();
    if let Err(error) = arguments.logging.init() {
        eprintln!("{error}");
        return std::process::ExitCode::FAILURE;
    }
    let result = tokio::select! { biased;
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Stopped waiting. Use status to inspect the daemon.");
            return std::process::ExitCode::from(130);
        },
        result = commands::run(arguments) => result,
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        // A closed reader (`status | head`) ends the output, not the command.
        Err(commands::Error::Output(error)) if error.kind() == std::io::ErrorKind::BrokenPipe => {
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(error = %error, "Local command failed");
            std::process::ExitCode::FAILURE
        }
    }
}
#[cfg(not(unix))]
fn main() {
    eprintln!("This CLI requires Unix local sockets.");
    std::process::exit(1);
}
