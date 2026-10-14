//! Minimal ISO-MP4 edit-list reader. symphonia's isomp4 reader is not gapless, so AAC priming (e.g. 1024 frames,
//! recorded as an `elst` media_time) would otherwise leak into the decoded audio (≈ 46 ms at 22.05 kHz).
//! We walk moov/trak/{tkhd,mdia/mdhd,edts/elst} for one track and return how much to skip/keep, in seconds.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditInfo {
    /// seconds of decoded audio to drop at the start (priming)
    pub skip_s: f64,
    /// presentation duration in seconds to keep after the skip (if the edit list defines it)
    pub keep_s: Option<f64>,
}

struct BoxHdr {
    kind: [u8; 4],
    payload: u64,
    end: u64,
}

fn read_box(f: &mut File, pos: u64, limit: u64) -> Option<BoxHdr> {
    if pos + 8 > limit {
        return None;
    }
    f.seek(SeekFrom::Start(pos)).ok()?;
    let mut h = [0u8; 8];
    f.read_exact(&mut h).ok()?;
    let mut size = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64;
    let mut hdr = 8;
    if size == 1 {
        let mut e = [0u8; 8];
        f.read_exact(&mut e).ok()?;
        size = u64::from_be_bytes(e);
        hdr = 16;
    } else if size == 0 {
        size = limit - pos;
    }
    if size < hdr || pos + size > limit {
        return None;
    }
    Some(BoxHdr {
        kind: [h[4], h[5], h[6], h[7]],
        payload: pos + hdr,
        end: pos + size,
    })
}

fn children(f: &mut File, parent: &BoxHdr) -> Vec<BoxHdr> {
    let mut v = Vec::new();
    let mut p = parent.payload;
    while let Some(b) = read_box(f, p, parent.end) {
        p = b.end;
        v.push(b);
    }
    v
}

fn read_bytes(f: &mut File, at: u64, n: usize) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(at)).ok()?;
    let mut b = vec![0u8; n];
    f.read_exact(&mut b).ok()?;
    Some(b)
}

pub fn audio_edit(path: &Path, track_id: u32) -> Option<EditInfo> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut top = Vec::new();
    let mut p = 0;
    while let Some(b) = read_box(&mut f, p, len) {
        p = b.end;
        top.push(b);
    }
    let moov = top.iter().find(|b| &b.kind == b"moov")?;
    let kids = children(&mut f, moov);
    let mvhd = kids.iter().find(|b| &b.kind == b"mvhd")?;
    let d = read_bytes(&mut f, mvhd.payload, 24)?;
    let movie_ts = if d[0] == 1 {
        u32::from_be_bytes([d[20], d[21], d[22], d[23]])
    } else {
        u32::from_be_bytes([d[12], d[13], d[14], d[15]])
    } as f64;
    for trak in kids.iter().filter(|b| &b.kind == b"trak") {
        let tk = children(&mut f, trak);
        let tkhd = tk.iter().find(|b| &b.kind == b"tkhd")?;
        let d = read_bytes(&mut f, tkhd.payload, 24)?;
        let id = if d[0] == 1 {
            u32::from_be_bytes([d[20], d[21], d[22], d[23]])
        } else {
            u32::from_be_bytes([d[12], d[13], d[14], d[15]])
        };
        if id != track_id {
            continue;
        }
        let mdia = tk.iter().find(|b| &b.kind == b"mdia")?;
        let md = children(&mut f, mdia);
        let mdhd = md.iter().find(|b| &b.kind == b"mdhd")?;
        let d = read_bytes(&mut f, mdhd.payload, 24)?;
        let media_ts = if d[0] == 1 {
            u32::from_be_bytes([d[20], d[21], d[22], d[23]])
        } else {
            u32::from_be_bytes([d[12], d[13], d[14], d[15]])
        } as f64;
        let edts = tk.iter().find(|b| &b.kind == b"edts")?;
        let elst = children(&mut f, edts)
            .into_iter()
            .find(|b| &b.kind == b"elst")?;
        let d = read_bytes(&mut f, elst.payload, (elst.end - elst.payload) as usize)?;
        let version = d[0];
        let n = u32::from_be_bytes([d[4], d[5], d[6], d[7]]) as usize;
        let mut off = 8;
        let mut skip = 0.0;
        let mut keep: Option<f64> = None;
        let mut first_real = true;
        for _ in 0..n {
            let (seg, mt, step) = if version == 1 {
                if d.len() < off + 20 {
                    return None;
                }
                (
                    u64::from_be_bytes(d[off..off + 8].try_into().ok()?),
                    i64::from_be_bytes(d[off + 8..off + 16].try_into().ok()?),
                    20,
                )
            } else {
                if d.len() < off + 12 {
                    return None;
                }
                (
                    u32::from_be_bytes(d[off..off + 4].try_into().ok()?) as u64,
                    i32::from_be_bytes(d[off + 4..off + 8].try_into().ok()?) as i64,
                    12,
                )
            };
            off += step + 4; // + media_rate integer/fraction
            if mt < 0 {
                continue; // empty edit (initial delay); ignore
            }
            if first_real {
                skip = mt as f64 / media_ts;
                first_real = false;
            }
            keep = Some(keep.unwrap_or(0.0) + seg as f64 / movie_ts);
        }
        if first_real {
            return None;
        }
        return Some(EditInfo {
            skip_s: skip,
            keep_s: keep.filter(|k| *k > 0.0),
        });
    }
    None
}
