//! Constants of the spec: ports (Table B2-1), register addresses (Table B8-2), codes (Table
//! B8-3), management commands and status codes (Tables A1-2, A1-3).

pub mod port {
    pub const DISCOVERY: u16 = 1024; // discovery, General, Set IP; discovery reply out
    pub const DDC_SPECIFIC: u16 = 1025; // in; High Priority status out
    pub const DUC_SPECIFIC: u16 = 1026; // in; microphone out (not offered)
    pub const HIGH_PRIORITY: u16 = 1027; // in; wideband out (from M2)
    pub const DUC_IQ: u16 = 1029;
    pub const DDC0_IQ: u16 = 1035; // out
    pub const REGMAP: u16 = 7250;
    pub const MGMT: u16 = 7251;
    /// Table B2-1: exactly these local ports on level 1.
    pub const LEVEL1: [u16; 8] = [1024, 1025, 1026, 1027, 1029, 1035, 7250, 7251];
}

pub const BOARD_TYPE: u8 = 200; // 0002
pub const P2_VERSION: u8 = 44; // B3.5
pub const ARM_MAGIC: u32 = 0x4737_5458; // "G7TX", B8.6
pub const DSP_CLOCK_HZ: u64 = 122_880_000; // B3.8

pub mod reg {
    pub const CAP_BASE: u16 = 0x0000;
    pub const CAP_LENGTH: u16 = 0xCF; // 207, record 1.1
    pub const CAP_REQUEST: u16 = 0x0100;
    pub const STREAM_STATUS: u16 = 0x0101;
    pub const LINK_STATUS: u16 = 0x0102;
    pub const WRITE_STATUS: u16 = 0x0103;
    pub const TX_ARM_TOKEN: u16 = 0x0200;
    pub const TX_ARM: u16 = 0x0201;
    pub const TX_ARM_STATE: u16 = 0x0202;
    pub const TX_STATE: u16 = 0x0203;
    pub const TX_MODE_CLASS: u16 = 0x0204;
    pub const KEYED_TIMEOUT: u16 = 0x0205;
    pub const WATCHDOG_STATE: u16 = 0x0206;
    pub const SAFETY_STATE: u16 = 0x0210;
    pub const FAULT_CODE: u16 = 0x0211;
    pub const FAULT_FLAGS: u16 = 0x0212;
    pub const FAULT_CLASS: u16 = 0x0213;
    pub const TRIP_COUNT: u16 = 0x0214;
    pub const FAULT_CLEAR: u16 = 0x0215;
    pub const LIMIT_STATUS: u16 = 0x0220;
    pub const LIMIT_INHIBIT: u16 = 0x0221;
    pub const LIMIT_TX_TIME: u16 = 0x0230;
    pub const LIMIT_FWD_POWER: u16 = 0x0231;
    pub const LIMIT_SWR: u16 = 0x0232;
    pub const LIMIT_PA_TEMP: u16 = 0x0233;
    pub const TLM_FIRST: u16 = 0x0300;
    pub const SENSOR_VALID: u16 = 0x0307;
    pub const ATU_TUNE: u16 = 0x0400;
    pub const ATU_STATE: u16 = 0x0401;
    pub const ANT_STATE: u16 = 0x0500;
    pub const PADDING: u16 = 0xFFFF;
}

/// Write reject reasons, `WRITE_STATUS` [7:0] (Table B8-3).
pub mod wr {
    pub const UNKNOWN_ADDRESS: u8 = 1;
    pub const READ_ONLY: u8 = 2;
    pub const NOT_OWNER: u8 = 3;
    pub const BAD_VALUE: u8 = 4;
    pub const LOOSER_LIMIT: u8 = 5;
    pub const BUSY: u8 = 6;
    pub const NOT_CLEARABLE: u8 = 7;
    pub const KEYED: u8 = 8;
    pub const NOT_ARMED: u8 = 9;
    pub const NOT_KEYABLE: u8 = 10;
    pub const NOT_CALIBRATED: u8 = 11;
}

/// Arm reject reasons, `TX_ARM_STATE` [15:8] (Table B8-3).
pub mod arm {
    pub const NONE: u8 = 0;
    pub const NOT_RUNNING: u8 = 2;
    pub const WATCHDOG_DISABLED: u8 = 3;
    pub const IN_FAULT: u8 = 4;
    pub const INHIBITED: u8 = 5;
    pub const PTT_ACTIVE: u8 = 6;
    pub const LEVEL_NOT_ACTIVE: u8 = 7;
    pub const MAINTENANCE: u8 = 8;
    pub const NO_MODE_CLASS: u8 = 9;
    pub const BAD_TOKEN: u8 = 10;
    pub const TX_NOT_IMPLEMENTED: u8 = 11;
    /// Transmit parameters of Annex P not set (record `tx` bit 4 = 0; 0035, 0036).
    pub const NOT_CALIBRATED: u8 = 12;
}

/// Drop reasons, `TX_ARM_STATE` [23:16] (Table B8-3, Table A3-1).
pub mod drop {
    pub const NONE: u8 = 0;
    pub const HOST_DISARM: u8 = 1;
    pub const RUN_CLEARED: u8 = 2;
    pub const WATCHDOG: u8 = 3;
    pub const TAKEOVER: u8 = 4;
    pub const RESET: u8 = 5;
    pub const FAULT: u8 = 6;
    pub const LINK_DOWN: u8 = 8;
    pub const SESSION_CHANGED: u8 = 9;
    pub const LEVEL_CHANGE: u8 = 10;
    pub const MAINTENANCE: u8 = 11;
    pub const REDISCOVERY: u8 = 12;
}

/// `TX_STATE` [23:16] unkey reasons (Table B8-2).
pub mod unkey {
    pub const RELEASED: u8 = 1;
    pub const KEYED_TIMEOUT: u8 = 2;
    pub const ARM_DROPPED: u8 = 5;
    pub const PA_DISABLED: u8 = 7;
    pub const CW_CAP: u8 = 9;
    pub const MODE_CLASS: u8 = 10;
}

/// Stream reasons, `STREAM_STATUS` [6:0] (Table B8-3).
pub mod sr {
    pub const NONE: u8 = 0;
    pub const NOT_ENABLED: u8 = 1;
    pub const NOT_OFFERED: u8 = 2;
    pub const RATE: u8 = 3;
    pub const SIZE: u8 = 4;
    pub const FORMAT: u8 = 5;
    pub const TIMESTAMP: u8 = 6;
    pub const LINK: u8 = 8;
    pub const RUN_CLEAR: u8 = 9;
    pub const UPDATE: u8 = 10;
    pub const VNA: u8 = 11;
    pub const EER: u8 = 12;
    pub const FREQ_ENCODING: u8 = 13;
    pub const TX_NOT_CALIBRATED: u8 = 14;
}

pub mod mgmt {
    pub const MAGIC: u32 = 0x4737_4D47; // "G7MG"
    pub const VERSION: (u8, u8) = (1, 0);
    pub const IDENTIFY: u16 = 0x0001;
    pub const GET_STATS: u16 = 0x0002;
    pub const GET_CHALLENGE: u16 = 0x0010;
    pub const SET_FIRST_SECRET: u16 = 0x0011;
    pub const CHANGE_SECRET: u16 = 0x0012;
    pub const END_WRITE_SESSION: u16 = 0x0013;
    pub const GET_CONFIG: u16 = 0x0020;
    pub const SET_CONFIG: u16 = 0x0021;
    pub const FACTORY_RESET: u16 = 0x0022;
    pub const SWITCH_LEVEL: u16 = 0x0023;
    pub const RESTART: u16 = 0x0024;
    pub const READ_LOG: u16 = 0x0030;
    pub const CLEAR_LOG: u16 = 0x0031;
    pub const UPDATE_BEGIN: u16 = 0x0040;
    pub const UPDATE_STATUS: u16 = 0x0045;
    pub const ROLLBACK: u16 = 0x0046;
    pub const RETURN_TO_STOCK: u16 = 0x0050;
    pub const ENTER_VENDOR_RECOVERY: u16 = 0x0051;
    pub const GET_CALIBRATION: u16 = 0x0060;
    pub const SET_CALIBRATION: u16 = 0x0061;
    pub const CLEAR_CALIBRATION: u16 = 0x0062;

    pub mod st {
        pub const OK: u16 = 0;
        pub const IN_PROGRESS: u16 = 1;
        pub const UNKNOWN_COMMAND: u16 = 16;
        pub const MALFORMED: u16 = 17;
        pub const VERSION: u16 = 18;
        pub const TOO_LARGE: u16 = 19;
        pub const AUTH_REQUIRED: u16 = 32;
        pub const AUTH_FAILED: u16 = 33;
        pub const RATE_LIMITED: u16 = 34;
        pub const BUSY: u16 = 35;
        pub const WINDOW_CLOSED: u16 = 36;
        pub const INVALID_CONFIG: u16 = 48;
        pub const NOT_ALLOWED: u16 = 49;
        pub const TOO_LARGE_IMAGE: u16 = 61;
        pub const NEEDS_CALIBRATION: u16 = 62;
        pub const NOT_SUPPORTED: u16 = 56;
        pub const SESSION_RUNS: u16 = 57;
        pub const UPDATE_IN_PROGRESS: u16 = 58;
        pub const ARMED: u16 = 50;
    }
}
