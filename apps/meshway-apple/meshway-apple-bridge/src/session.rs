//! Bounded byte transport only. Node's existing codec owns typed IPC decoding.
use crate::{BridgeError, runtime};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex, mpsc},
};
use tokio_util::sync::CancellationToken;

#[uniffi::export(callback_interface)]
pub trait FrameSink: Send + Sync {
    /// Host consumes/ACKs one frame before returning. Must return within five
    /// seconds even when its XPC client disappears; false closes the session.
    fn on_frame(&self, frame: Vec<u8>) -> bool;
}

#[derive(uniffi::Object)]
pub struct Session {
    sent: std::sync::atomic::AtomicBool,
    input: mpsc::Sender<Vec<u8>>,
    owner: Arc<Task>,
}

pub(crate) struct Task {
    stop: CancellationToken,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Task {
    pub(crate) fn is_finished(&self) -> bool {
        self.task.try_lock().is_ok_and(|task| {
            task.as_ref()
                .is_none_or(tokio::task::JoinHandle::is_finished)
        })
    }
    pub(crate) async fn close(&self) {
        self.stop.cancel();
        let mut task = self.task.lock().await;
        if let Some(task) = task.take() {
            let _ = task.await;
        }
    }
}

impl Session {
    pub(crate) fn owner(&self) -> Arc<Task> {
        self.owner.clone()
    }
    #[cfg(test)]
    pub(crate) fn is_closed(&self) -> bool {
        self.owner.stop.is_cancelled()
    }
    pub(crate) fn open(
        ipc: &meshway_node::ipc::Handle,
        stop: CancellationToken,
        sink: Box<dyn FrameSink>,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<Arc<Self>, BridgeError> {
        let (client, server) = tokio::io::duplex(meshway_protocol::ipc::MAX_MESSAGE_BYTES + 4);
        ipc.submit(server).map_err(|_| BridgeError::Capacity)?;
        let (input, mut incoming) = mpsc::channel::<Vec<u8>>(1);
        let stopped = stop.clone();
        let sink: Arc<dyn FrameSink> = sink.into();
        let task = runtime()?.spawn(async move {
            let _permit = permit;
            let (mut reader, mut writer) = tokio::io::split(client);
            let request = async {
                let frame = incoming.recv().await?;
                writer.write_all(&frame).await.ok()
            };
            let sent = tokio::select! {
                _ = stopped.cancelled() => None,
                sent = request => sent,
            };
            if sent.is_none() {
                stopped.cancel();
                return;
            }
            loop {
                let read = async {
                    let Ok(length) = reader.read_u32().await else {
                        return None;
                    };
                    if length == 0 || length as usize > meshway_protocol::ipc::MAX_MESSAGE_BYTES {
                        return None;
                    }
                    let mut frame = vec![0; length as usize + 4];
                    frame[..4].copy_from_slice(&length.to_be_bytes());
                    if reader.read_exact(&mut frame[4..]).await.is_err() {
                        return None;
                    }
                    Some(frame)
                };
                let frame = tokio::select! {
                    _ = stopped.cancelled() => None,
                    frame = read => frame,
                };
                let Some(frame) = frame else {
                    break;
                };
                let sink = sink.clone();
                // Blocking XPC ACK stays off the Node's async executor.
                // Join it even during cancellation; its host has a deadline.
                let sent = tokio::task::spawn_blocking(move || sink.on_frame(frame)).await;
                if !matches!(sent, Ok(true)) || stopped.is_cancelled() {
                    break;
                }
            }
            stopped.cancel();
        });
        Ok(Arc::new(Self {
            sent: std::sync::atomic::AtomicBool::new(false),
            input,
            owner: Arc::new(Task {
                stop,
                task: Mutex::new(Some(task)),
            }),
        }))
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Session {
    pub fn send(&self, frame: Vec<u8>) -> Result<(), BridgeError> {
        if self.owner.stop.is_cancelled() {
            return Err(BridgeError::Stopped);
        }
        let prefix: [u8; 4] = frame
            .get(..4)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(BridgeError::Frame)?;
        let length = u32::from_be_bytes(prefix) as usize;
        if length == 0
            || length > meshway_protocol::ipc::MAX_MESSAGE_BYTES
            || frame.len() != length + 4
        {
            return Err(BridgeError::Frame);
        }
        if self.sent.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Err(BridgeError::Frame);
        }
        self.input
            .try_send(frame)
            .map_err(|_| BridgeError::Capacity)
    }
    pub async fn close(&self) {
        self.owner.close().await;
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.owner.stop.cancel();
    }
}
