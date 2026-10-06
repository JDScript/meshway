//! Layered performance fixture. Plain is deliberately unauthenticated IP over UDP.
#![forbid(unsafe_code)]

pub mod plain;

use clap::ValueEnum;
use serde::Serialize;
use std::net::{Ipv4Addr, SocketAddr};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Reserved stages fail explicitly until their real domain implementation exists.
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
            },
        )
    }
}

/// Shared node boundary for future real SecureNode/Manager drivers. The harness
/// supplies a namespace and explicit peer; no fixture changes host networking.
#[derive(Clone, Debug)]
pub struct NodeOptions {
    pub stage: Stage,
    pub role: NodeRole,
    pub bind: SocketAddr,
    pub peer: SocketAddr,
    pub mtu: usize,
    pub batch: usize,
}

impl NodeOptions {
    pub fn validate(&self) -> Result<(), Error> {
        if !matches!(self.stage, Stage::Plain) {
            return Err("requested stage is not implemented; no substitute is run".into());
        }
        if !(1280..=65000).contains(&self.mtu) || !(1..=1024).contains(&self.batch) {
            return Err("MTU must be 1280..=65000 and batch 1..=1024".into());
        }
        Ok(())
    }
}
