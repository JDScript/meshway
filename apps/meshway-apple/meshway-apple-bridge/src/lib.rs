//! Apple ABI only. Node policy, packet and control owners live in meshway-node.
#![cfg(target_os = "macos")]
#![deny(unsafe_code)]

// UniFFI emits extern ABI functions and pointer conversion glue. This module
// permits generated-code unsafe. The separate fd module audits one integer-only
// dup syscall and adoption of its successful fresh result.
#[allow(unsafe_code)]
mod abi {
    uniffi::setup_scaffolding!();
}
pub use abi::UniFfiTag;

use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

mod session;
pub use session::{FrameSink, Session};
mod settings;
pub use settings::{NetworkSettings, Route, TunnelSettingsSink};
#[allow(unsafe_code)]
mod fd;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum BridgeError {
    #[error("{message}")]
    Startup { message: String },
    #[error("Node is stopped")]
    Stopped,
    #[error("IPC capacity exhausted")]
    Capacity,
    #[error("Invalid IPC frame")]
    Frame,
}

#[derive(uniffi::Record)]
pub struct StartConfig {
    pub state_directory: String,
    pub server_origin: String,
    pub hostname: Option<String>,
}

#[uniffi::export(callback_interface)]
pub trait LogSink: Send + Sync {
    fn line(&self, line: String);
}

struct LogWriter(Arc<dyn LogSink>, Vec<u8>);
impl std::io::Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.1.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Drop for LogWriter {
    fn drop(&mut self) {
        if !self.1.is_empty() {
            self.0.line(String::from_utf8_lossy(&self.1).into_owned());
        }
    }
}

fn runtime() -> Result<&'static tokio::runtime::Runtime, BridgeError> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|message| BridgeError::Startup {
            message: message.clone(),
        })
}

#[derive(uniffi::Object)]
pub struct NodeHandle {
    apple: Option<Arc<meshway_node::host::apple::Apple>>,
    ipc: meshway_node::ipc::Handle,
    stop: CancellationToken,
    task: Mutex<Option<tokio::task::JoinHandle<Result<(), &'static str>>>>,
    sessions: std::sync::Mutex<Vec<Arc<session::Task>>>,
    slots: Arc<tokio::sync::Semaphore>,
}

/// Explicit control-only entry for validating the embedding and IPC transport.
/// It reports controlOnly and never claims packet attachment or applied.
#[uniffi::export]
pub fn start_control_only(
    config: StartConfig,
    logs: Box<dyn LogSink>,
) -> Result<Arc<NodeHandle>, BridgeError> {
    open(config, logs, None)
}

/// The provider must have exclusive packet I/O on this utun. We duplicate its
/// descriptor; NE retains ownership of the original and performs no flow reads.
#[uniffi::export]
pub fn start(
    config: StartConfig,
    logs: Box<dyn LogSink>,
    settings: Box<dyn TunnelSettingsSink>,
    utun_fd: i32,
    utun_name: String,
) -> Result<Arc<NodeHandle>, BridgeError> {
    let descriptor = fd::duplicate(utun_fd).map_err(|error| BridgeError::Startup {
        message: error.to_string(),
    })?;
    let sink = Arc::new(settings::Sink(Arc::from(settings)));
    let apple =
        meshway_node::host::apple::Apple::new(descriptor, utun_name, sink).map_err(|error| {
            BridgeError::Startup {
                message: error.to_string(),
            }
        })?;
    open(config, logs, Some(Arc::new(apple)))
}

fn open(
    config: StartConfig,
    logs: Box<dyn LogSink>,
    apple: Option<Arc<meshway_node::host::apple::Apple>>,
) -> Result<Arc<NodeHandle>, BridgeError> {
    static LOGGING: OnceLock<Result<(), String>> = OnceLock::new();
    LOGGING
        .get_or_init(|| {
            let logs: Arc<dyn LogSink> = logs.into();
            meshway_logging::Options {
                filter: "info".into(),
                format: meshway_logging::Format::Json,
            }
            .init_with_writer(move || LogWriter(logs.clone(), Vec::new()), false)
            .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|message| BridgeError::Startup {
            message: message.clone(),
        })?;
    let settings = meshway_node::Settings::new(
        config.state_directory.into(),
        &config.server_origin,
        config.hostname.as_deref(),
    )
    .map_err(|message| BridgeError::Startup {
        message: message.into(),
    })?
    .with_network_mode(if apple.is_some() {
        meshway_protocol::ipc::NetworkMode::Tun
    } else {
        meshway_protocol::ipc::NetworkMode::ControlOnly
    });
    let platform: Arc<dyn meshway_node::host::Platform> = match &apple {
        Some(apple) => apple.clone(),
        None => Arc::new(meshway_node::host::ControlOnly),
    };
    let runtime = runtime()?;
    let node = runtime
        .block_on(meshway_node::Node::open(settings, platform))
        .map_err(|message| BridgeError::Startup {
            message: message.into(),
        })?;
    let ipc = node.ipc();
    let stop = CancellationToken::new();
    let shutdown = stop.clone();
    let task = runtime.spawn(async move { node.run(shutdown.cancelled()).await });
    Ok(Arc::new(NodeHandle {
        apple,
        ipc,
        stop,
        task: Mutex::new(Some(task)),
        sessions: std::sync::Mutex::new(Vec::new()),
        slots: Arc::new(tokio::sync::Semaphore::new(8)),
    }))
}

#[uniffi::export(async_runtime = "tokio")]
impl NodeHandle {
    pub fn path_changed(&self, interface: Option<String>, path: bool) -> Result<(), BridgeError> {
        if self.stop.is_cancelled() {
            return Err(BridgeError::Stopped);
        }
        if let Some(apple) = &self.apple {
            apple
                .path_changed(interface, path)
                .map_err(|error| BridgeError::Startup {
                    message: error.to_string(),
                })?;
        }
        Ok(())
    }
    pub async fn stop(&self) -> Result<(), BridgeError> {
        let sessions: Vec<_> = {
            let sessions = self.sessions.lock().map_err(|_| BridgeError::Stopped)?;
            self.stop.cancel();
            self.slots.close();
            sessions.clone()
        };
        for session in sessions {
            session.close().await;
        }
        // Keep the mutex through join so concurrent stops also wait for exit.
        let mut task = self.task.lock().await;
        if let Some(running) = task.as_mut() {
            let result = running.await;
            task.take();
            result
                .map_err(|error| BridgeError::Startup {
                    message: error.to_string(),
                })?
                .map_err(|message| BridgeError::Startup {
                    message: message.into(),
                })?;
        }
        Ok(())
    }
    pub fn open_ipc_session(&self, sink: Box<dyn FrameSink>) -> Result<Arc<Session>, BridgeError> {
        let mut sessions = self.sessions.lock().map_err(|_| BridgeError::Stopped)?;
        if self.stop.is_cancelled() {
            return Err(BridgeError::Stopped);
        }
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| BridgeError::Capacity)?;
        let session = Session::open(&self.ipc, self.stop.child_token(), sink, permit)?;
        sessions.retain(|session| !session.is_finished());
        sessions.push(session.owner());
        Ok(session)
    }
}
impl Drop for NodeHandle {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

#[cfg(test)]
mod tests;
