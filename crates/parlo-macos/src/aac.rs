//! M4A/AAC export through AudioToolbox `ExtAudioFile` (BRIEF §9.4). DRAFTED IN THE CLOUD, UNCHECKED ON A MAC.
//!
//! Plain C declarations (AudioToolbox is a stable C API; no objc2 wrapper crate needed). The WAV is read through
//! `parlo_core::audio::wav::PcmFile` in 4096-frame blocks, so memory stays bounded for any length.
//! Flow: create an M4A file with an AAC destination format → set the client format to 16-bit PCM → set the converter's
//! bit rate → write blocks → dispose (which finalizes the `moov` atom).

use core_foundation::base::TCFType;
use core_foundation::url::CFURL;
use core_foundation_sys::url::CFURLRef;
use parlo_core::audio::wav::PcmFile;
use parlo_core::error::{Error, Result};
use parlo_core::platform::AacEncoder;
use std::ffi::c_void;
use std::path::Path;
use std::ptr;

type OSStatus = i32;
type ExtAudioFileRef = *mut c_void;
type AudioConverterRef = *mut c_void;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct AudioStreamBasicDescription {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
struct AudioBuffer {
    number_channels: u32,
    data_byte_size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    number_buffers: u32,
    buffers: [AudioBuffer; 1],
}

const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}
const K_AUDIO_FILE_M4A_TYPE: u32 = fourcc(b"m4af");
const K_AUDIO_FORMAT_MPEG4_AAC: u32 = fourcc(b"aac ");
const K_AUDIO_FORMAT_LINEAR_PCM: u32 = fourcc(b"lpcm");
const K_AUDIO_FORMAT_FLAG_IS_SIGNED_INTEGER: u32 = 1 << 2;
const K_AUDIO_FORMAT_FLAG_IS_PACKED: u32 = 1 << 3;
const K_EXT_AUDIO_FILE_PROPERTY_CLIENT_DATA_FORMAT: u32 = fourcc(b"cfmt");
const K_EXT_AUDIO_FILE_PROPERTY_AUDIO_CONVERTER: u32 = fourcc(b"acnv");
const K_EXT_AUDIO_FILE_PROPERTY_CONVERTER_CONFIG: u32 = fourcc(b"accf");
const K_AUDIO_CONVERTER_ENCODE_BIT_RATE: u32 = fourcc(b"brat");
const K_AUDIO_FILE_FLAGS_ERASE_FILE: u32 = 1;

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn ExtAudioFileCreateWithURL(
        url: CFURLRef,
        file_type: u32,
        desc: *const AudioStreamBasicDescription,
        layout: *const c_void,
        flags: u32,
        out: *mut ExtAudioFileRef,
    ) -> OSStatus;
    fn ExtAudioFileSetProperty(
        file: ExtAudioFileRef,
        property: u32,
        size: u32,
        data: *const c_void,
    ) -> OSStatus;
    fn ExtAudioFileGetProperty(
        file: ExtAudioFileRef,
        property: u32,
        size: *mut u32,
        data: *mut c_void,
    ) -> OSStatus;
    fn ExtAudioFileWrite(
        file: ExtAudioFileRef,
        frames: u32,
        data: *const AudioBufferList,
    ) -> OSStatus;
    fn ExtAudioFileDispose(file: ExtAudioFileRef) -> OSStatus;
    fn AudioConverterSetProperty(
        conv: AudioConverterRef,
        property: u32,
        size: u32,
        data: *const c_void,
    ) -> OSStatus;
}

struct FileGuard(ExtAudioFileRef);
impl FileGuard {
    /// Dispose explicitly so the status (the `moov` finalization) can be reported.
    fn finish(mut self) -> OSStatus {
        let f = std::mem::replace(&mut self.0, ptr::null_mut());
        if f.is_null() {
            0
        } else {
            unsafe { ExtAudioFileDispose(f) }
        }
    }
}
impl Drop for FileGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { ExtAudioFileDispose(self.0) };
        }
    }
}

fn check(what: &str, status: OSStatus) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "AudioToolbox {what} failed (OSStatus {status})"
        )))
    }
}

pub struct AudioToolboxAac;

impl AacEncoder for AudioToolboxAac {
    fn is_available(&self) -> bool {
        true
    }

    fn encode_wav_to_m4a(&self, wav: &Path, dest: &Path, bitrate_kbps: u32) -> Result<()> {
        let pcm = PcmFile::open(wav)?;
        let rate = pcm.sample_rate as f64;
        let url = CFURL::from_path(dest, false)
            .ok_or_else(|| Error::Other("invalid M4A destination path".into()))?;

        // destination: AAC, mono
        let dst = AudioStreamBasicDescription {
            sample_rate: rate,
            format_id: K_AUDIO_FORMAT_MPEG4_AAC,
            channels_per_frame: 1,
            ..Default::default()
        };
        // client (what we feed in): 16-bit signed mono PCM
        let client = AudioStreamBasicDescription {
            sample_rate: rate,
            format_id: K_AUDIO_FORMAT_LINEAR_PCM,
            format_flags: K_AUDIO_FORMAT_FLAG_IS_SIGNED_INTEGER | K_AUDIO_FORMAT_FLAG_IS_PACKED,
            bytes_per_packet: 2,
            frames_per_packet: 1,
            bytes_per_frame: 2,
            channels_per_frame: 1,
            bits_per_channel: 16,
            reserved: 0,
        };

        let mut file: ExtAudioFileRef = ptr::null_mut();
        check("ExtAudioFileCreateWithURL", unsafe {
            ExtAudioFileCreateWithURL(
                url.as_concrete_TypeRef(),
                K_AUDIO_FILE_M4A_TYPE,
                &dst,
                ptr::null(),
                K_AUDIO_FILE_FLAGS_ERASE_FILE,
                &mut file,
            )
        })?;
        let guard = FileGuard(file);
        check("set client format", unsafe {
            ExtAudioFileSetProperty(
                file,
                K_EXT_AUDIO_FILE_PROPERTY_CLIENT_DATA_FORMAT,
                std::mem::size_of::<AudioStreamBasicDescription>() as u32,
                &client as *const _ as *const c_void,
            )
        })?;

        // bit rate: set on the file's converter, then re-apply the (null) converter config so the change takes effect
        let mut conv: AudioConverterRef = ptr::null_mut();
        let mut sz = std::mem::size_of::<AudioConverterRef>() as u32;
        check("get converter", unsafe {
            ExtAudioFileGetProperty(
                file,
                K_EXT_AUDIO_FILE_PROPERTY_AUDIO_CONVERTER,
                &mut sz,
                &mut conv as *mut _ as *mut c_void,
            )
        })?;
        let bps: u32 = bitrate_kbps * 1000;
        check("set bit rate", unsafe {
            AudioConverterSetProperty(
                conv,
                K_AUDIO_CONVERTER_ENCODE_BIT_RATE,
                4,
                &bps as *const u32 as *const c_void,
            )
        })?;
        let null_cfg: *const c_void = ptr::null();
        // (best effort: some OS versions reject a null config; the bit rate is already set on the converter)
        let _ = unsafe {
            ExtAudioFileSetProperty(
                file,
                K_EXT_AUDIO_FILE_PROPERTY_CONVERTER_CONFIG,
                std::mem::size_of::<*const c_void>() as u32,
                &null_cfg as *const _ as *const c_void,
            )
        };

        const BLOCK: usize = 4096;
        let mut f32buf = Vec::new();
        let mut i16buf: Vec<i16> = Vec::with_capacity(BLOCK);
        let mut pos = 0u64;
        while pos < pcm.frames {
            let n = BLOCK.min((pcm.frames - pos) as usize);
            pcm.read_f32(pos, n, &mut f32buf)?;
            i16buf.clear();
            i16buf.extend(
                f32buf
                    .iter()
                    .map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16),
            );
            let list = AudioBufferList {
                number_buffers: 1,
                buffers: [AudioBuffer {
                    number_channels: 1,
                    data_byte_size: (i16buf.len() * 2) as u32,
                    data: i16buf.as_mut_ptr() as *mut c_void,
                }],
            };
            check("ExtAudioFileWrite", unsafe {
                ExtAudioFileWrite(file, i16buf.len() as u32, &list)
            })?;
            pos += n as u64;
        }
        check("ExtAudioFileDispose", guard.finish())
    }
}
