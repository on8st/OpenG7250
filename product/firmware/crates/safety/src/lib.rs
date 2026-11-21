// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Safety core of the G7250 main firmware: the RF safe state ([HIS] HIS 85) and the transmit
//! gate (spec A3; decision 0007; decision 0032 and ON8ST 2026-11-20: every TX request is
//! rejected with `NOT_CALIBRATED` until the measured values are in the spec).
#![cfg_attr(not(test), no_std)]

pub mod safe_state;
pub mod tx;
