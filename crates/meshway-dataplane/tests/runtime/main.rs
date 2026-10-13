//! Real HTTP enrollment and the real netmap compiler feed Control-issued
//! netmap snapshots into real SecureNodes over native TCP: A→R→B, with B also
//! an IPv4 exit. These tests prove admission, wire authentication and the
//! original reply path; not daemon execution or TUN I/O.
#![cfg(unix)]

mod netmap_diagnostic;
mod netmap_exit;
mod netmap_flow;
mod netmap_lifecycle;
mod netmap_mtu;
mod netmap_paths;
mod support;

use support::packet;
