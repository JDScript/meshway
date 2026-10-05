//! Hosted anonymous Binding service. App owns sockets/tasks; discovery metadata
//! is explicit deployment configuration, independent of management HTTP binding.
use crate::AppError;
use meshway_stun::{Service, ServiceConfig, ServiceStats};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{net::UdpSocket, task::JoinSet};
use tokio_util::sync::CancellationToken;

pub(crate) fn bind(addresses: &[SocketAddr]) -> Result<Vec<UdpSocket>, AppError> {
    addresses
        .iter()
        .map(|address| {
            let bind = || -> io::Result<UdpSocket> {
                let socket = Socket::new(
                    Domain::for_address(*address),
                    Type::DGRAM,
                    Some(Protocol::UDP),
                )?;
                socket.set_nonblocking(true)?;
                if address.is_ipv6() {
                    socket.set_only_v6(true)?;
                }
                socket.bind(&(*address).into())?;
                UdpSocket::from_std(socket.into())
            };
            bind().map_err(|_| AppError::Stun)
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct Hosted {
    stop: CancellationToken,
    tasks: JoinSet<io::Result<ServiceStats>>,
    failed: Arc<AtomicBool>,
}
#[derive(Clone)]
pub(crate) struct Health {
    stop: CancellationToken,
    failed: Arc<AtomicBool>,
}
impl Health {
    pub(crate) fn ready(&self) -> bool {
        !self.stop.is_cancelled() && !self.failed.load(Ordering::Acquire)
    }
}
struct Completion(Health);
impl Drop for Completion {
    fn drop(&mut self) {
        if !self.0.stop.is_cancelled() {
            self.0.failed.store(true, Ordering::Release);
        }
    }
}
impl Hosted {
    pub(crate) fn health(&self) -> Health {
        Health {
            stop: self.stop.clone(),
            failed: self.failed.clone(),
        }
    }
    pub fn start(sockets: Vec<UdpSocket>) -> Result<Self, AppError> {
        let mut owner = Self::default();
        for socket in sockets {
            let address = socket.local_addr().map_err(|_| AppError::Stun)?;
            let service = Service::new(socket, ServiceConfig::default())
                .map_err(|_| AppError::InvalidSettings)?;
            let stop = owner.stop.clone();
            let failed = owner.failed.clone();
            let completion = Completion(owner.health());
            owner.tasks.spawn(async move {
                let _completion = completion;
                let result = service.run(stop.clone()).await;
                if !stop.is_cancelled() {
                    failed.store(true, Ordering::Release);
                    tracing::error!(%address, "Hosted STUN listener stopped unexpectedly");
                }
                result
            });
            tracing::info!(%address, "Hosted STUN listener ready");
        }
        Ok(owner)
    }
    pub fn ready(&self) -> bool {
        self.health().ready()
    }
    pub async fn shutdown(mut self) -> Result<(), AppError> {
        self.stop.cancel();
        let mut failed = self.failed.load(Ordering::Acquire);
        while let Some(result) = self.tasks.join_next().await {
            failed |= !matches!(result, Ok(Ok(_)));
        }
        if failed { Err(AppError::Stun) } else { Ok(()) }
    }
}
impl Drop for Hosted {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_deployment_has_no_udp_sockets_or_service_tasks() {
        let sockets = bind(&[]).unwrap();
        assert!(sockets.is_empty());
        let owner = Hosted::start(sockets).unwrap();
        assert!(owner.tasks.is_empty());
        assert!(owner.ready());
        let health = owner.health();
        owner.shutdown().await.unwrap();
        assert!(!health.ready());
    }

    #[tokio::test]
    async fn hosted_binding_uses_the_core_and_shutdown_releases_the_owned_socket() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let owner = Hosted::start(vec![socket]).unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let transaction = meshway_stun::TransactionId::random().unwrap();
        client
            .send_to(
                &meshway_stun::encode_request(transaction, meshway_stun::Fingerprint::Include),
                address,
            )
            .await
            .unwrap();
        let mut buffer = [0; meshway_stun::MAX_DATAGRAM_SIZE];
        let (length, source) = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            client.recv_from(&mut buffer),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            meshway_stun::parse_response(transaction, address, source, &buffer[..length])
                .unwrap()
                .socket_addr(),
            client.local_addr().unwrap()
        );
        owner.shutdown().await.unwrap();
        let _rebound = UdpSocket::bind(address).await.unwrap();
    }

    #[tokio::test]
    async fn unexpected_service_exit_invalidates_cloned_metadata_health() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut owner = Hosted::start(vec![socket]).unwrap();
        let health = owner.health();
        owner.tasks.abort_all();
        assert!(owner.tasks.join_next().await.unwrap().is_err());
        assert!(!health.ready());
        assert_eq!(owner.shutdown().await, Err(AppError::Stun));
    }
}
