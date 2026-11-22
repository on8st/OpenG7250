// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Management channel, UDP port 7251 (spec A1, Annex M; spec/draft 56197e4).
//!
//! The shared protocol crate of 0037: the firmware (the radio end) and `g7250ctl` (the host
//! end) both use this module, so it encodes and decodes both directions. It is written from the
//! spec only; the reference model is independent and never used here (0037 item 3).
//!
//! No allocation: messages are built into and parsed from caller buffers.

pub mod auth;
pub mod payload;
pub mod tlv;

/// Local UDP port of the management channel (A1.3).
pub const PORT: u16 = 7251;
/// Header magic "G7MG" (Table A1-1).
pub const MAGIC: u32 = 0x4737_4D47;
/// Protocol version of this crate (Table A1-1 bytes 4, 5).
pub const VERSION_MAJOR: u8 = 1;
pub const VERSION_MINOR: u8 = 0;
/// Header length, bytes 0-17 (Table A1-1).
pub const HEADER_LEN: usize = 18;
/// HMAC-SHA256 trailer length (Table A1-1).
pub const TRAILER_LEN: usize = 32;
/// Largest request datagram (A1.4).
pub const MAX_REQUEST: usize = 1000;
/// Largest reply datagram (A1.4).
pub const MAX_REPLY: usize = 1400;
/// Data bytes in one update block (A1.4; Table M-1 `UPDATE_BLOCK`).
pub const BLOCK_DATA: usize = 512;

pub const TYPE_REQUEST: u8 = 1;
pub const TYPE_REPLY: u8 = 2;
pub const FLAG_AUTH: u8 = 0x01;
pub const FLAG_MORE: u8 = 0x02;

/// Commands (Table A1-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Command {
    Identify = 0x0001,
    GetStats = 0x0002,
    GetChallenge = 0x0010,
    SetFirstSecret = 0x0011,
    ChangeSecret = 0x0012,
    EndWriteSession = 0x0013,
    GetConfig = 0x0020,
    SetConfig = 0x0021,
    FactoryReset = 0x0022,
    SwitchLevel = 0x0023,
    Restart = 0x0024,
    ReadLog = 0x0030,
    ClearLog = 0x0031,
    UpdateBegin = 0x0040,
    UpdateBlock = 0x0041,
    UpdateVerify = 0x0042,
    UpdateActivate = 0x0043,
    UpdateAbort = 0x0044,
    UpdateStatus = 0x0045,
    Rollback = 0x0046,
    ReturnToStock = 0x0050,
    EnterVendorRecovery = 0x0051,
    GetCalibration = 0x0060,
    SetCalibration = 0x0061,
    ClearCalibration = 0x0062,
}

impl Command {
    pub const ALL: [Command; 25] = [
        Command::Identify,
        Command::GetStats,
        Command::GetChallenge,
        Command::SetFirstSecret,
        Command::ChangeSecret,
        Command::EndWriteSession,
        Command::GetConfig,
        Command::SetConfig,
        Command::FactoryReset,
        Command::SwitchLevel,
        Command::Restart,
        Command::ReadLog,
        Command::ClearLog,
        Command::UpdateBegin,
        Command::UpdateBlock,
        Command::UpdateVerify,
        Command::UpdateActivate,
        Command::UpdateAbort,
        Command::UpdateStatus,
        Command::Rollback,
        Command::ReturnToStock,
        Command::EnterVendorRecovery,
        Command::GetCalibration,
        Command::SetCalibration,
        Command::ClearCalibration,
    ];

    pub fn from_u16(v: u16) -> Option<Command> {
        Self::ALL.iter().copied().find(|c| *c as u16 == v)
    }

    /// Whether the command needs authentication (Table A1-2 column "Auth").
    /// `SET_FIRST_SECRET` is "no (window only)": no trailer, but only while the window is open.
    pub fn needs_auth(self) -> bool {
        !matches!(
            self,
            Command::Identify
                | Command::GetStats
                | Command::GetChallenge
                | Command::SetFirstSecret
                | Command::GetConfig
                | Command::ReadLog
                | Command::UpdateStatus
                | Command::GetCalibration
        )
    }
}

/// Status codes (Table A1-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u16)]
pub enum Status {
    Ok = 0,
    InProgress = 1,
    UnknownCommand = 16,
    Malformed = 17,
    UnsupportedVersion = 18,
    TooLarge = 19,
    AuthRequired = 32,
    AuthFailed = 33,
    RateLimited = 34,
    Busy = 35,
    FirstSecretClosed = 36,
    InvalidConfig = 48,
    NotAllowed = 49,
    Armed = 50,
    WrongHardware = 51,
    OlderImage = 52,
    BadBlockCrc = 53,
    BlockOutOfOrder = 54,
    VerificationFailed = 55,
    NotSupported = 56,
    P2SessionRuns = 57,
    UpdateInProgress = 58,
    RfPresent = 59,
    OneWayInProgress = 60,
    ImageTooLarge = 61,
    NeedsCalibration = 62,
    /// Image linked for the running slot (A1.8; 0043).
    RunningSlot = 63,
    /// Nothing taken: room made in the store; send the block again as a new request (A1.8 "Store
    /// full during a transfer"; 0067 addendum).
    RoomMade = 64,
}

impl Status {
    pub fn from_u16(v: u16) -> Option<Status> {
        use Status::*;
        [
            Ok, InProgress, UnknownCommand, Malformed, UnsupportedVersion, TooLarge, AuthRequired, AuthFailed,
            RateLimited, Busy, FirstSecretClosed, InvalidConfig, NotAllowed, Armed, WrongHardware, OlderImage,
            BadBlockCrc, BlockOutOfOrder, VerificationFailed, NotSupported, P2SessionRuns, UpdateInProgress,
            RfPresent, OneWayInProgress, ImageTooLarge, NeedsCalibration, RunningSlot, RoomMade,
        ]
        .into_iter()
        .find(|s| *s as u16 == v)
    }

    /// Rank in the order of A1.3 "Where several codes apply" (lower wins): 19 for a declared
    /// length above 1000, 17, 18, 16, 19 for any other size limit, then 32, 33, 34, 36, then 35,
    /// 58, 57, 50, then the command-specific codes. Framing codes are decided by [`check`]; this
    /// rank orders the codes a command handler may collect.
    pub fn rank(self) -> u8 {
        use Status::*;
        match self {
            Malformed => 1,
            UnsupportedVersion => 2,
            UnknownCommand => 3,
            TooLarge => 4,
            AuthRequired => 5,
            AuthFailed => 6,
            RateLimited => 7,
            FirstSecretClosed => 8,
            Busy => 9,
            UpdateInProgress => 10,
            P2SessionRuns => 11,
            Armed => 12,
            Ok | InProgress => 255,
            _ => 20,
        }
    }

    /// The code a reply carries when several apply.
    pub fn first(codes: &[Status]) -> Status {
        codes.iter().copied().min_by_key(|s| s.rank()).unwrap_or(Status::Ok)
    }
}

/// A parsed header (Table A1-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub major: u8,
    pub minor: u8,
    pub msg_type: u8,
    pub flags: u8,
    pub request_id: u32,
    pub command: u16,
    pub status: u16,
    pub payload_len: u16,
}

impl Header {
    pub fn authenticated(&self) -> bool {
        self.flags & FLAG_AUTH != 0
    }

    pub fn write(&self, out: &mut [u8]) {
        out[0..4].copy_from_slice(&MAGIC.to_be_bytes());
        out[4] = self.major;
        out[5] = self.minor;
        out[6] = self.msg_type;
        out[7] = self.flags;
        out[8..12].copy_from_slice(&self.request_id.to_be_bytes());
        out[12..14].copy_from_slice(&self.command.to_be_bytes());
        out[14..16].copy_from_slice(&self.status.to_be_bytes());
        out[16..18].copy_from_slice(&self.payload_len.to_be_bytes());
    }

    fn read(b: &[u8]) -> Header {
        Header {
            major: b[4],
            minor: b[5],
            msg_type: b[6],
            flags: b[7],
            request_id: u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
            command: u16::from_be_bytes([b[12], b[13]]),
            status: u16::from_be_bytes([b[14], b[15]]),
            payload_len: u16::from_be_bytes([b[16], b[17]]),
        }
    }
}

/// What the radio does with a received datagram, before any command logic (A1.4).
#[derive(Debug, PartialEq, Eq)]
pub enum Incoming<'a> {
    /// Drop without reply: another magic, or too short to carry a request id.
    Drop,
    /// Drop without reply and count: right magic, but not a request (A1.4; 0022 §73 item 20).
    DropNotRequest,
    /// Answer with this status only, echoing the request id (and the command if known).
    Answer { request_id: u32, command: u16, status: Status },
    /// A well-framed request for the command layer.
    Request(Request<'a>),
}

/// A well-framed request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request<'a> {
    pub header: Header,
    /// Bytes 0 .. 17 + N (the part the HMAC covers after the challenge).
    pub signed: &'a [u8],
    pub payload: &'a [u8],
    pub trailer: Option<&'a [u8; TRAILER_LEN]>,
    /// The command, if known; an unknown command is answered with 16 by the caller.
    pub command: Option<Command>,
}

/// Frame checks of A1.4, in the order of A1.3 "Where several codes apply".
pub fn check(d: &[u8]) -> Incoming<'_> {
    if d.len() < 4 || u32::from_be_bytes([d[0], d[1], d[2], d[3]]) != MAGIC {
        return Incoming::Drop;
    }
    if d.len() >= 7 && d[6] != TYPE_REQUEST {
        return Incoming::DropNotRequest;
    }
    if d.len() < 12 {
        return Incoming::Drop;
    }
    let request_id = u32::from_be_bytes([d[8], d[9], d[10], d[11]]);
    let command = if d.len() >= 14 { u16::from_be_bytes([d[12], d[13]]) } else { 0 };
    if d.len() < HEADER_LEN {
        return Incoming::Answer { request_id, command, status: Status::Malformed };
    }
    let h = Header::read(d);
    let auth = h.authenticated();
    let declared = HEADER_LEN + h.payload_len as usize + if auth { TRAILER_LEN } else { 0 };
    // 19 first, decided from the header alone (A1.4, first bullet).
    if declared > MAX_REQUEST {
        return Incoming::Answer { request_id, command, status: Status::TooLarge };
    }
    if d.len() != declared {
        return Incoming::Answer { request_id, command, status: Status::Malformed };
    }
    if h.major != VERSION_MAJOR {
        return Incoming::Answer { request_id, command, status: Status::UnsupportedVersion };
    }
    let Some(cmd) = Command::from_u16(h.command) else {
        return Incoming::Answer { request_id, command, status: Status::UnknownCommand };
    };
    let end = HEADER_LEN + h.payload_len as usize;
    let trailer = if auth { Some(d[end..end + TRAILER_LEN].try_into().unwrap()) } else { None };
    Incoming::Request(Request { header: h, signed: &d[..end], payload: &d[HEADER_LEN..end], trailer, command: Some(cmd) })
}

/// Error while building a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// Message longer than the buffer, or than the limit for its type (1000 / 1400 bytes).
    TooLarge,
}

/// Build a request (host end). With `auth = Some((secret, challenge))` the HMAC trailer is
/// appended (Annex M "Authentication"). Returns the datagram length.
pub fn build_request(
    out: &mut [u8],
    request_id: u32,
    command: Command,
    payload: &[u8],
    auth: Option<(&[u8; 32], &[u8; 16])>,
) -> Result<usize, BuildError> {
    let len = HEADER_LEN + payload.len() + if auth.is_some() { TRAILER_LEN } else { 0 };
    if len > MAX_REQUEST || len > out.len() || payload.len() > u16::MAX as usize {
        return Err(BuildError::TooLarge);
    }
    Header {
        major: VERSION_MAJOR,
        minor: VERSION_MINOR,
        msg_type: TYPE_REQUEST,
        flags: if auth.is_some() { FLAG_AUTH } else { 0 },
        request_id,
        command: command as u16,
        status: 0,
        payload_len: payload.len() as u16,
    }
    .write(out);
    out[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);
    if let Some((secret, challenge)) = auth {
        let end = HEADER_LEN + payload.len();
        let mac = auth::trailer(secret, challenge, &out[..end]);
        out[end..end + TRAILER_LEN].copy_from_slice(&mac);
    }
    Ok(len)
}

/// Build a reply (radio end). Returns the datagram length.
pub fn build_reply(out: &mut [u8], request_id: u32, command: u16, status: Status, payload: &[u8]) -> Result<usize, BuildError> {
    let len = HEADER_LEN + payload.len();
    if len > MAX_REPLY || len > out.len() {
        return Err(BuildError::TooLarge);
    }
    Header {
        major: VERSION_MAJOR,
        minor: VERSION_MINOR,
        msg_type: TYPE_REPLY,
        flags: 0,
        request_id,
        command,
        status: status as u16,
        payload_len: payload.len() as u16,
    }
    .write(out);
    out[HEADER_LEN..len].copy_from_slice(payload);
    Ok(len)
}

/// A parsed reply (host end).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply<'a> {
    pub header: Header,
    pub status: Option<Status>,
    pub payload: &'a [u8],
}

/// Parse a reply (host end). `None` for anything that is not a well-formed reply.
pub fn parse_reply(d: &[u8]) -> Option<Reply<'_>> {
    if d.len() < HEADER_LEN || u32::from_be_bytes([d[0], d[1], d[2], d[3]]) != MAGIC || d[6] != TYPE_REPLY {
        return None;
    }
    let h = Header::read(d);
    let end = HEADER_LEN + h.payload_len as usize;
    if d.len() != end {
        return None;
    }
    Some(Reply { header: h, status: Status::from_u16(h.status), payload: &d[HEADER_LEN..end] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(cmd: u16, payload: &[u8], flags: u8) -> std::vec::Vec<u8> {
        let mut v = std::vec![0u8; HEADER_LEN + payload.len()];
        Header { major: 1, minor: 0, msg_type: 1, flags, request_id: 0xA1B2C3D4, command: cmd, status: 0, payload_len: payload.len() as u16 }.write(&mut v);
        v[HEADER_LEN..].copy_from_slice(payload);
        v
    }

    #[test]
    fn header_layout_table_a1_1() {
        let v = req(0x0001, &[], 0);
        assert_eq!(&v[0..4], b"G7MG");
        assert_eq!(&v[4..8], &[1, 0, 1, 0]);
        assert_eq!(&v[8..12], &[0xA1, 0xB2, 0xC3, 0xD4]);
        assert_eq!(&v[12..18], &[0, 1, 0, 0, 0, 0]);
        match check(&v) {
            Incoming::Request(r) => assert_eq!(r.command, Some(Command::Identify)),
            x => panic!("{x:?}"),
        }
    }

    #[test]
    fn drops_and_framing_order() {
        assert_eq!(check(b"XXXX"), Incoming::Drop);
        let mut r = req(1, &[], 0);
        r[6] = TYPE_REPLY;
        assert_eq!(check(&r), Incoming::DropNotRequest);
        // Right magic, under 12 bytes: dropped; 12-17 bytes: 17.
        let r = req(1, &[], 0);
        assert_eq!(check(&r[..11]), Incoming::Drop);
        assert!(matches!(check(&r[..14]), Incoming::Answer { status: Status::Malformed, request_id: 0xA1B2C3D4, .. }));
        // Declared length above 1000: 19, even if the datagram is short (A1.4).
        let mut r = req(1, &[], 0);
        r[16..18].copy_from_slice(&983u16.to_be_bytes()); // 18 + 983 = 1001
        assert!(matches!(check(&r), Incoming::Answer { status: Status::TooLarge, .. }));
        // Declared 1000 exactly, but a different datagram length: 17.
        r[16..18].copy_from_slice(&982u16.to_be_bytes());
        assert!(matches!(check(&r), Incoming::Answer { status: Status::Malformed, .. }));
        // With the auth flag, the trailer counts toward the 1000.
        let mut r = req(1, &[], FLAG_AUTH);
        r[16..18].copy_from_slice(&951u16.to_be_bytes()); // 18 + 951 + 32 = 1001
        assert!(matches!(check(&r), Incoming::Answer { status: Status::TooLarge, .. }));
        // Unknown major: 18; unknown command: 16.
        let mut r = req(1, &[], 0);
        r[4] = 2;
        assert!(matches!(check(&r), Incoming::Answer { status: Status::UnsupportedVersion, .. }));
        let r = req(0x7777, &[], 0);
        assert!(matches!(check(&r), Incoming::Answer { status: Status::UnknownCommand, command: 0x7777, .. }));
        // Length mismatch beats unknown major (17 before 18).
        let mut r = req(1, &[1, 2], 0);
        r[4] = 2;
        assert!(matches!(check(&r[..19]), Incoming::Answer { status: Status::Malformed, .. }));
    }

    #[test]
    fn status_priority() {
        use Status::*;
        assert_eq!(Status::first(&[Armed, P2SessionRuns]), P2SessionRuns);
        assert_eq!(Status::first(&[Armed, UpdateInProgress, P2SessionRuns]), UpdateInProgress);
        assert_eq!(Status::first(&[NotSupported, Armed]), Armed);
        assert_eq!(Status::first(&[Busy, AuthFailed]), AuthFailed);
        assert_eq!(Status::first(&[NeedsCalibration, Busy]), Busy);
        assert_eq!(Status::first(&[]), Ok);
    }

    #[test]
    fn request_reply_roundtrip() {
        let mut b = [0u8; MAX_REPLY];
        let n = build_request(&mut b, 9, Command::GetConfig, &[], None).unwrap();
        let Incoming::Request(r) = check(&b[..n]) else { panic!() };
        assert_eq!((r.header.request_id, r.command, r.trailer), (9, Some(Command::GetConfig), None));
        let n = build_reply(&mut b, 9, Command::GetConfig as u16, Status::Ok, &[1, 1, 0]).unwrap();
        let p = parse_reply(&b[..n]).unwrap();
        assert_eq!((p.header.request_id, p.status, p.payload), (9, Some(Status::Ok), &[1u8, 1, 0][..]));
        assert_eq!(build_reply(&mut b, 1, 1, Status::Ok, &[0u8; 1383]), Err(BuildError::TooLarge));
        assert_eq!(build_request(&mut b, 1, Command::SetConfig, &[0u8; 983], None), Err(BuildError::TooLarge));
    }

    #[test]
    fn auth_column() {
        assert!(!Command::Identify.needs_auth());
        assert!(!Command::UpdateStatus.needs_auth());
        assert!(!Command::SetFirstSecret.needs_auth());
        assert!(Command::SetConfig.needs_auth());
        assert!(Command::ReturnToStock.needs_auth());
        assert!(Command::SetCalibration.needs_auth());
        for c in Command::ALL {
            assert_eq!(Command::from_u16(c as u16), Some(c));
        }
    }
}
