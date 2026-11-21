// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Protocol code of the G7250 main firmware, host-testable (`cargo test`). Normative source:
//! `product/spec/spec/network-interface.md` (cited by section, for example B3.3).
#![cfg_attr(not(test), no_std)]

pub mod dhcp;
pub mod discovery;
pub mod netcfg;
pub mod ratelimit;
