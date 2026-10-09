use super::{
    backend::Backend, child::Fence, command::Commands, objects, routing::Routing, storage::Store,
};
use crate::{Error, Result};
use meshway_private_storage::Directory;
use rtnetlink::packet_route::link::LinkAttribute;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, RwLock},
};
use tokio_util::sync::CancellationToken;

pub(in crate::linux::kernel) async fn recover(
    handle: &rtnetlink::Handle,
    directory: Arc<Directory>,
    limit: usize,
) -> Result<()> {
    let (mut store, prior) = Store::open(directory).await?;
    let Some(mut ledger) = prior.filter(|ledger| !ledger.removed) else {
        return Ok(());
    };
    // Descriptor ownership is never inferred from a journal name/index. A live
    // device at this identity belongs to a surviving descriptor or a conflict.
    let links = super::super::collect(handle.link().get().execute(), limit).await?;
    if links.iter().any(|link| link.header.index == ledger.program.interface || link.attributes.iter().any(|attribute| matches!(attribute, LinkAttribute::IfName(name) if name == &ledger.tun_name))) {
        #[cfg(test)] eprintln!("exit recovery live interface: {} index={}", ledger.tun_name, ledger.program.interface);
        return Err(Error::Conflict);
    }
    let mut commands = Commands::recorded(Fence {
        directory: store.directory.clone(),
        installation: ledger.program.installation.clone(),
    });
    match objects::present(&mut commands, &ledger.program).await? {
        0 if ledger.rules.is_empty()
            && ledger.routes.is_empty()
            && (ledger.projections.is_empty()
                || (ledger.closing && ledger.projections.iter().all(|flow| flow.retired))) =>
        {
            ledger.removed = true;
            ledger.pending = false;
            return store.persist(&ledger).await;
        }
        2 => {}
        _ => return Err(Error::Conflict),
    }
    if ledger.receipt.is_none() {
        if !ledger.projections.is_empty() || !ledger.rules.is_empty() || !ledger.routes.is_empty() {
            return Err(Error::Journal);
        }
        ledger.receipt = Some(objects::recover_create(&mut commands, &ledger.program).await?);
        store.persist(&ledger).await?;
    }
    let routing = Routing {
        handle: handle.clone(),
        expected: Arc::new(Mutex::new(Default::default())),
        limit,
        start: 0,
        end: 0,
    };
    let mut backend = Backend {
        store,
        ledger,
        commands,
        routing,
        view: Arc::new(RwLock::new(BTreeMap::new())),
        live: BTreeMap::new(),
        local: Vec::new(),
        stop: CancellationToken::new(),
    };
    backend.close(false).await
}
