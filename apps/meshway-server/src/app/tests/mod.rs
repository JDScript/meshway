//! Whole-application tests: real App, storage and services, no fake Node or
//! execution payloads. They inspect private storage through the executor
//! handle, which is why they live in the crate rather than in `tests/`.
use super::*;

/// State reports never compile. The previous publication chain still reads
/// reported endpoints and exit readiness when another stable input compiles;
/// this stands in for such an input.
async fn recompile(app: &App, network: meshway_model::NetworkId) {
    let sealer = app.sealer();
    app.executor
        .handle()
        .write(move |tx| {
            crate::store::execution::publish(
                tx,
                &sealer,
                network,
                "refresh",
                crate::store::execution::now()?,
            )
            .map(|_| ())
        })
        .await
        .unwrap();
}

mod cache;
mod commit;
mod connectivity;
mod demand_renewal;
mod discovery;
mod lifecycle;
mod node;
mod refresh;
mod resources;
mod session;
mod state_scope;
