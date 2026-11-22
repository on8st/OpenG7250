// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! `g7250ctl`: the reference host implementation of the G7250 management channel (spec part A,
//! A1.1a; decisions 0026, 0037, 0070). The message layouts come from the protocol crate shared
//! with the firmware (`g7250-proto`); this crate holds the host side: transport and
//! retransmission, discovery, and output.

pub mod args;
pub mod discover;
pub mod reads;
pub mod render;
pub mod secret;
pub mod stock;
pub mod transport;
pub mod update;
pub mod writes;

use std::net::{SocketAddr, ToSocketAddrs};

/// Process exit codes (README "Exit codes").
pub mod exit {
    /// Done; the radio answered with status 0.
    pub const OK: u8 = 0;
    /// The radio answered with a status other than 0.
    pub const STATUS: u8 = 1;
    /// Bad command line (clap uses 2 as well).
    pub const USAGE: u8 = 2;
    /// No reply, or no radio found.
    pub const NO_REPLY: u8 = 3;
    /// A reply that could not be read, or a local error.
    pub const ERROR: u8 = 4;
}

/// Resolve `host` or `host:port` to an IPv4 socket address, with `default_port` when the port is
/// left out. The management channel is IPv4 only ([CH394]; A1.11).
pub fn resolve(addr: &str, default_port: u16) -> Result<SocketAddr, String> {
    let with_port = if addr.parse::<std::net::Ipv4Addr>().is_ok() || !addr.contains(':') {
        format!("{addr}:{default_port}")
    } else {
        addr.to_string()
    };
    with_port
        .to_socket_addrs()
        .map_err(|e| format!("{addr}: {e}"))?
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| format!("{addr}: no IPv4 address"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_forms() {
        assert_eq!(resolve("192.168.1.20", 7251).unwrap(), "192.168.1.20:7251".parse().unwrap());
        assert_eq!(resolve("192.168.1.20:9000", 7251).unwrap(), "192.168.1.20:9000".parse().unwrap());
        assert_eq!(resolve("localhost:9000", 7251).unwrap().port(), 9000);
        assert!(resolve("192.168.1.20:notaport", 7251).is_err());
    }
}
