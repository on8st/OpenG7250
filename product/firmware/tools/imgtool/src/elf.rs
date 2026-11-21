// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Minimal ELF32 little-endian program-header reader (enough for ARM images).

pub struct Segment {
    pub offset: u32,
    pub vaddr: u32,
    pub paddr: u32,
    pub filesz: u32,
    pub memsz: u32,
}

const PT_LOAD: u32 = 1;

fn u16_at(b: &[u8], o: usize) -> Result<u16, String> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| "truncated ELF".into())
}
fn u32_at(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| "truncated ELF".into())
}

pub fn load_segments(b: &[u8]) -> Result<Vec<Segment>, String> {
    if b.len() < 52 || &b[0..4] != b"\x7fELF" {
        return Err("not an ELF file".into());
    }
    if b[4] != 1 || b[5] != 1 {
        return Err("not ELF32 little-endian".into());
    }
    if u16_at(b, 18)? != 40 {
        return Err("not an ARM ELF".into());
    }
    let phoff = u32_at(b, 28)? as usize;
    let phentsize = u16_at(b, 42)? as usize;
    let phnum = u16_at(b, 44)? as usize;
    let mut v = Vec::new();
    for i in 0..phnum {
        let o = phoff + i * phentsize;
        if u32_at(b, o)? != PT_LOAD {
            continue;
        }
        v.push(Segment {
            offset: u32_at(b, o + 4)?,
            vaddr: u32_at(b, o + 8)?,
            paddr: u32_at(b, o + 12)?,
            filesz: u32_at(b, o + 16)?,
            memsz: u32_at(b, o + 20)?,
        });
    }
    Ok(v)
}

/// One ELF32 ARM executable holding the load segments of all inputs, entry point of the first.
/// No section headers; loaders that work from program headers need none.
pub fn merge(inputs: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let entry = u32_at(&inputs[0], 24)?;
    let mut segs: Vec<(Segment, &[u8])> = Vec::new();
    for f in inputs {
        for s in load_segments(f)? {
            let data = f.get(s.offset as usize..(s.offset + s.filesz) as usize).ok_or("segment beyond end of file")?;
            segs.push((s, data));
        }
    }
    // Flash bytes of different inputs must not overlap.
    for (i, a) in segs.iter().enumerate() {
        for b in &segs[i + 1..] {
            let (x, y) = (&a.0, &b.0);
            if x.filesz > 0 && y.filesz > 0 && x.paddr < y.paddr + y.filesz && y.paddr < x.paddr + x.filesz {
                return Err(format!("segments at {:#010x} and {:#010x} overlap", x.paddr, y.paddr));
            }
        }
    }
    let phnum = segs.len();
    let phoff = 52usize;
    let mut out = vec![0u8; phoff + 32 * phnum];
    out[0..4].copy_from_slice(b"\x7fELF");
    out[4] = 1; // ELFCLASS32
    out[5] = 1; // little-endian
    out[6] = 1; // EV_CURRENT
    out[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    out[18..20].copy_from_slice(&40u16.to_le_bytes()); // EM_ARM
    out[20..24].copy_from_slice(&1u32.to_le_bytes());
    out[24..28].copy_from_slice(&entry.to_le_bytes());
    out[28..32].copy_from_slice(&(phoff as u32).to_le_bytes());
    out[36..40].copy_from_slice(&0x0500_0400u32.to_le_bytes()); // EABI v5, hard-float
    out[40..42].copy_from_slice(&52u16.to_le_bytes());
    out[42..44].copy_from_slice(&32u16.to_le_bytes());
    out[44..46].copy_from_slice(&(phnum as u16).to_le_bytes());
    out[46..48].copy_from_slice(&40u16.to_le_bytes());
    for (i, (s, data)) in segs.iter().enumerate() {
        let off = (out.len() + 3) & !3;
        out.resize(off, 0);
        out.extend_from_slice(data);
        let ph = [PT_LOAD, off as u32, s.vaddr, s.paddr, s.filesz, s.memsz, 0x7, 4];
        let o = phoff + 32 * i;
        for (k, v) in ph.iter().enumerate() {
            out[o + 4 * k..o + 4 * k + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    Ok(out)
}
