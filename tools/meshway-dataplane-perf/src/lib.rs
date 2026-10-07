//! Layered performance fixture. Plain is deliberately unauthenticated IP over UDP.
#![forbid(unsafe_code)]

pub mod material;
pub mod plain;
pub mod secure;

use clap::ValueEnum;
use serde::Serialize;
use std::net::{Ipv4Addr, SocketAddr};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Explicit layers; each stage uses its production domain implementation.
#[derive(Clone, Copy, Debug, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Plain,
    Secure,
    Manager,
}

#[derive(Clone, Copy, Debug, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum NodeRole {
    A,
    B,
    R,
}

impl NodeRole {
    pub fn mesh_address(self) -> Ipv4Addr {
        Ipv4Addr::new(
            100,
            127,
            0,
            match self {
                Self::A => 1,
                Self::B => 2,
                Self::R => 3,
            },
        )
    }
}

/// Shared node boundary for plain, SecureNode and Manager drivers. The harness
/// supplies a namespace and explicit peer; no fixture changes host networking.
#[derive(Clone, Debug)]
pub struct NodeOptions {
    pub stage: Stage,
    pub role: NodeRole,
    pub bind: SocketAddr,
    pub peer: SocketAddr,
    pub mtu: usize,
    pub batch: usize,
    /// Zero runs stateless crypto inline; nonzero uses the same production pool.
    pub workers: usize,
}

impl NodeOptions {
    pub fn validate(&self) -> Result<(), Error> {
        if self.workers > 256 {
            return Err("worker count must be 0..=256".into());
        }
        if matches!(self.stage, Stage::Plain) && matches!(self.role, NodeRole::R) {
            return Err("plain stage has no relay".into());
        }
        if !(1280..=65000).contains(&self.mtu) || !(1..=1024).contains(&self.batch) {
            return Err("MTU must be 1280..=65000 and batch 1..=1024".into());
        }
        Ok(())
    }
}
