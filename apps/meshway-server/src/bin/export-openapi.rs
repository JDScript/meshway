#![forbid(unsafe_code)]

use std::{io::Write, process::ExitCode};

fn main() -> ExitCode {
    let result =
        serde_json::to_writer_pretty(std::io::stdout().lock(), &meshway_server::http::openapi())
            .and_then(|()| {
                std::io::stdout()
                    .lock()
                    .write_all(b"\n")
                    .map_err(serde_json::Error::io)
            });
    if result.is_err() {
        eprintln!("OpenAPI export failed.");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
