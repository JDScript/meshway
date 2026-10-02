//! Compatibility adapter for the original CLI-shaped public run methods.
use super::{SessionControl, SessionEvent};

pub(super) struct Context {
    pub(super) control: SessionControl,
    signal: tokio::task::JoinHandle<()>,
}
impl Context {
    pub(super) fn new() -> Self {
        let cancellation = tokio_util::sync::CancellationToken::new();
        let signal_token = cancellation.clone();
        let signal = tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            signal_token.cancel();
        });
        let (events, mut receive) = tokio::sync::mpsc::channel(32);
        std::thread::Builder::new()
            .name("meshway-cli-events".into())
            .spawn(move || {
                while let Some(event) = receive.blocking_recv() {
                    match event {
                        SessionEvent::Receipt(receipt) => println!("{receipt}"),
                        SessionEvent::Retry { message, delay } => {
                            eprintln!("{message}; retrying in {}s", delay.as_secs())
                        }
                    }
                }
            })
            .expect("CLI event adapter thread");
        Self {
            control: SessionControl {
                cancellation,
                events,
            },
            signal,
        }
    }
}
impl Drop for Context {
    fn drop(&mut self) {
        self.signal.abort();
    }
}
