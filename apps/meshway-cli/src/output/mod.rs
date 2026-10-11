//! Every command prints either human text (styled only on a TTY) or, with
//! `--json`, one typed JSON document (`ping`: one event per line).
use meshway_protocol::ipc::Status;
use std::io::{self, Write};

mod format;
pub(crate) mod netcheck;
pub(crate) mod ping;
pub(crate) mod status;
mod table;
mod term;
pub(crate) use term::Term;

pub(crate) fn json(output: &mut impl Write, value: &impl serde::Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    writeln!(output)
}

/// One status snapshot; a TTY watch redraws the screen in place.
pub(crate) fn status(status: &Status, json: bool, watch: bool) -> io::Result<()> {
    let mut output = io::stdout().lock();
    if json {
        self::json(&mut output, status)?;
        return output.flush();
    }
    let term = Term::stdout();
    if watch && term.rich {
        write!(output, "\u{1b}[H\u{1b}[2J")?;
    }
    self::status::write(&mut output, &term, status, chrono::Utc::now())?;
    if watch && !term.rich {
        writeln!(output)?;
    }
    output.flush()
}

/// The Node after a confirmed self-deletion.
pub(crate) fn logout(before: &Status, after: &Status, json: bool) -> io::Result<()> {
    let mut output = io::stdout().lock();
    if json {
        return self::json(&mut output, after);
    }
    let term = Term::stdout();
    let name = before
        .node
        .as_ref()
        .map(|node| node.hostname.clone())
        .or_else(|| {
            before
                .identity
                .as_ref()
                .map(|identity| identity.node_id.to_string())
        })
        .unwrap_or_else(|| "This Node".into());
    let network = before
        .identity
        .as_ref()
        .map(|identity| format!(" network {}", identity.network_id))
        .unwrap_or_default();
    writeln!(
        output,
        "{} {name} left{network}; the Server deleted it and its local identity is removed.",
        term.paint(term::Role::Good, "Logged out:")
    )?;
    writeln!(
        output,
        "Joining again needs a new enrollment: meshway up (browser) or meshway up --enrollment-key."
    )
}

/// Build and protocol identity; CLI and daemon of one release share it.
pub(crate) fn version(json: bool) -> io::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let git = env!("MESHWAY_BUILD_GIT");
    let target = env!("MESHWAY_BUILD_TARGET");
    let ipc = meshway_protocol::ipc::VERSION;
    let wire = meshway_dataplane::secure::WIRE_VERSION;
    let compiler = meshway_configuration::netmap::NETMAP_COMPILER_VERSION;
    let mut output = io::stdout().lock();
    if json {
        self::json(
            &mut output,
            &serde_json::json!({
                "version": version,
                "git": git,
                "target": target,
                "ipc": ipc,
                "wire": wire,
                "compilerVersion": compiler,
            }),
        )
    } else {
        writeln!(output, "meshway {version} ({git})")?;
        writeln!(output, "target:   {target}")?;
        writeln!(output, "ipc:      {ipc}")?;
        writeln!(output, "wire:     {wire}")?;
        writeln!(output, "compiler: {compiler}")
    }
}
