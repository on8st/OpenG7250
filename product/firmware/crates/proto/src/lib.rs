// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Protocol code of the G7250 main firmware, host-testable (`cargo test`). Normative source:
//! `product/spec/spec/network-interface.md` (cited by section, for example B3.3).
//!
//! This is the protocol crate shared by the firmware and `g7250ctl` (0037): `no_std`, no
//! allocation, both directions of every message. It is written from the spec only; the
//! reference model (`product/refmodel/`) is independent and never used here.
#![cfg_attr(not(test), no_std)]

pub mod dhcp;
pub mod discovery;
pub mod ipv4ll;
pub mod mgmt;
pub mod netcfg;
pub mod ratelimit;
