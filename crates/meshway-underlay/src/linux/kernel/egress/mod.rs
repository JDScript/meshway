mod backend;
pub(in crate::linux::kernel) mod mss;
mod owner;
pub(super) mod plan;
mod program;
mod storage;
#[cfg(test)]
mod tests;

pub use owner::EgressCapture;
pub(super) use owner::Owner;

use crate::{Error, Result};
use std::sync::{Arc, Mutex};

/// Close any previous positive selector before the base owner allocates or
/// verifies its RPDB. The durable OUTPUT union remains negative until explicit
/// authorized capture installation or down.
pub(super) async fn recover(
    handle: &rtnetlink::Handle,
    directory: Arc<meshway_private_storage::Directory>,
    limit: usize,
    remove: bool,
) -> Result<()> {
    let (store, prior) = storage::Store::open(directory).await?;
    let Some(ledger) = prior.filter(|ledger| !ledger.removed) else {
        return Ok(());
    };
    let commands = super::tool::command::Commands::recorded(super::tool::child::Fence {
        directory: store.directory.clone(),
        installation: ledger.program.installation.clone(),
    });
    let mut backend = backend::Backend {
        store,
        ledger,
        commands,
        handle: handle.clone(),
        expected: Arc::new(Mutex::new(super::monitor::Expected::default())),
        limit,
        start: 0,
        monitored: false,
    };
    backend.initialize().await?;
    backend.pause().await?;
    backend.remove_rules().await?;
    if remove {
        let _ = super::exit::objects::apply(
            &mut backend.commands,
            &mss::batch(&backend.ledger.program.installation, None),
        )
        .await;
        backend.retire().await?;
    }
    if backend.ledger.pending_gate.is_some() {
        return Err(Error::Journal);
    }
    Ok(())
}
