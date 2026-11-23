// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Radio-side state of the G7250 main firmware, host-testable. The management server lives
//! here, not in the shared protocol crate: only the radio end keeps challenges, the repeat cache
//! and the limits (0037: the protocol crate is shared with g7250ctl).
#![cfg_attr(not(test), no_std)]

pub mod ddc;
pub mod dhcp_client;
pub mod duc_fifo;
pub mod gaps;
pub mod calset;
pub mod mgmt;
pub mod record;
pub mod regmap;
pub mod session;
pub mod stream;
