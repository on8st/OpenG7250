// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The 32-byte management secret (A1.10; Annex M "Authentication").
//!
//! Rules of this tool: the secret is read from a file or from the environment, never from the
//! command line (where other users and the shell history see it); it is never printed or
//! logged; `Debug` shows nothing of it. A file holds 64 hex digits; surrounding whitespace is
//! ignored. A generated secret is written to a new file (never over an existing one), with
//! owner-only permissions on Unix, before it is sent to the radio.

use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::Path;

use g7250_proto::mgmt::auth::SECRET_LEN;

/// Environment variable that may hold the secret as 64 hex digits.
pub const ENV: &str = "G7250_SECRET";

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub [u8; SECRET_LEN]);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Best effort; not a guarantee against copies the compiler made.
        for b in self.0.iter_mut() {
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

impl Secret {
    pub fn bytes(&self) -> &[u8; SECRET_LEN] {
        &self.0
    }

    /// Parse 64 hex digits (whitespace around them ignored). The error never quotes the input.
    pub fn from_hex(s: &str) -> Result<Secret, String> {
        let s = s.trim();
        if s.len() != 2 * SECRET_LEN || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("a secret is {} hex digits", 2 * SECRET_LEN));
        }
        let mut b = [0u8; SECRET_LEN];
        for (i, out) in b.iter_mut().enumerate() {
            *out = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| "bad hex digit".to_string())?;
        }
        Ok(Secret(b))
    }

    pub fn from_file(path: &Path) -> Result<Secret, String> {
        let s = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Secret::from_hex(&s).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// `--secret-file` if given, else the environment variable.
    pub fn load(file: Option<&Path>) -> Result<Secret, String> {
        match file {
            Some(p) => Secret::from_file(p),
            None => match std::env::var(ENV) {
                Ok(v) => Secret::from_hex(&v).map_err(|e| format!("{ENV}: {e}")),
                Err(_) => Err(format!("this command needs the secret: --secret-file <file> or {ENV}")),
            },
        }
    }

    /// 32 bytes from the operating system's random source.
    pub fn generate() -> Result<Secret, String> {
        let mut b = [0u8; SECRET_LEN];
        getrandom::fill(&mut b).map_err(|e| format!("no random source: {e}"))?;
        Ok(Secret(b))
    }

    fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Write to a new file (refusing an existing one), owner read/write only on Unix. Elsewhere
    /// (Windows) the file takes the access rights of its folder: see [`rights_warning`].
    pub fn write_new(&self, path: &Path) -> Result<(), String> {
        let mut o = fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        f.write_all(format!("{}\n", self.hex()).as_bytes())
            .and_then(|_| f.sync_all())
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The warning for a new secret file on a system where the tool cannot restrict its rights
/// (anything but Unix: on Windows it takes the rights of its folder). `None` on Unix (mode 0600).
pub fn rights_warning(path: &Path) -> Option<String> {
    if cfg!(unix) { None } else { Some(not_restricted(path)) }
}

fn not_restricted(path: &Path) -> String {
    format!(
        "warning: {}: on this system the file takes the access rights of its folder; keep it in a folder only you can read (for example under your user profile), not in a shared one",
        path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_debug() {
        let h = "00112233445566778899aabbccddeeff00112233445566778899AABBCCDDEEFF";
        let s = Secret::from_hex(&format!("  {h}\n")).unwrap();
        assert_eq!(s.0[1], 0x11);
        assert_eq!(s.0[31], 0xFF);
        assert_eq!(format!("{s:?}"), "Secret(..)");
        let e = Secret::from_hex("abc123").unwrap_err();
        assert!(!e.contains("abc123"), "the error does not quote the input");
        assert!(Secret::from_hex(&h.replace('0', "g")).is_err());
    }

    #[test]
    fn generated_file_roundtrip_and_no_overwrite() {
        let dir = std::env::temp_dir().join(format!("g7250ctl-secret-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("radio.secret");
        let _ = fs::remove_file(&p);
        let s = Secret::generate().unwrap();
        assert_ne!(s.0, [0; 32]);
        s.write_new(&p).unwrap();
        assert_eq!(Secret::from_file(&p).unwrap(), s);
        assert!(s.write_new(&p).is_err(), "an existing file is never overwritten");
        // The warning is for systems other than Unix; its text is checked here as well.
        assert_eq!(rights_warning(&p), None);
        let w = not_restricted(Path::new("secret.hex"));
        assert!(w.starts_with("warning: secret.hex: on this system the file takes the access rights of its folder"), "{w}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
