//! AVFoundation fallback decoder (`AVAssetReader`) for files symphonia can't handle (BRIEF §7.1, §5.7).
//! DRAFTED IN THE CLOUD, UNCHECKED ON A MAC. The reader converts to 16 kHz mono 16-bit PCM itself, so the output goes
//! straight into the WAV that `audio::prepare` hands to the STT pipeline. Streaming: one sample buffer at a time.
//!
//! Core Media's block-buffer accessors are declared by hand (stable C API) to avoid depending on the shape of the
//! generated bindings for CF types.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_av_foundation::{AVAssetReader, AVAssetReaderTrackOutput, AVURLAsset};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};
use parlo_core::audio::wav::WavWriter;
use parlo_core::error::{Error, Result};
use parlo_core::platform::FallbackDecoder;
use std::ffi::c_void;
use std::path::Path;

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMSampleBufferGetDataBuffer(sbuf: *const c_void) -> *const c_void;
    fn CMBlockBufferGetDataLength(buf: *const c_void) -> usize;
    fn CMBlockBufferCopyDataBytes(
        buf: *const c_void,
        offset: usize,
        len: usize,
        dest: *mut c_void,
    ) -> i32;
}

const K_AUDIO_FORMAT_LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");

#[derive(Debug)]
pub struct AvFoundationDecoder;

impl FallbackDecoder for AvFoundationDecoder {
    fn decode_to_wav16k(&self, src: &Path, dest: &Path) -> Result<bool> {
        let path = src
            .to_str()
            .ok_or_else(|| Error::Other("path is not UTF-8".into()))?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };
        // audio tracks (synchronous accessor is fine on a worker thread; deprecated on macOS 15 but still present)
        #[allow(deprecated)]
        let tracks = unsafe { asset.tracksWithMediaType(objc2_av_foundation::AVMediaTypeAudio.expect("AVMediaTypeAudio")) };
        let Some(track) = tracks.firstObject() else {
            return Ok(false);
        };

        let keys: Vec<Retained<NSString>> = [
            "AVFormatIDKey",
            "AVSampleRateKey",
            "AVNumberOfChannelsKey",
            "AVLinearPCMBitDepthKey",
            "AVLinearPCMIsFloatKey",
            "AVLinearPCMIsBigEndianKey",
            "AVLinearPCMIsNonInterleaved",
        ]
        .iter()
        .map(|k| NSString::from_str(k))
        .collect();
        let vals: Vec<Retained<NSNumber>> = vec![
            NSNumber::new_u32(K_AUDIO_FORMAT_LINEAR_PCM),
            NSNumber::new_u32(16_000),
            NSNumber::new_u32(1),
            NSNumber::new_u32(16),
            NSNumber::new_bool(false),
            NSNumber::new_bool(false),
            NSNumber::new_bool(false),
        ];
        let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
        let val_refs: Vec<&AnyObject> = vals.iter().map(|v| &***v as &AnyObject).collect();
        let settings: Retained<NSDictionary<NSString, AnyObject>> =
            NSDictionary::from_slices(&key_refs, &val_refs);

        let reader = unsafe { AVAssetReader::assetReaderWithAsset_error(&asset) }
            .map_err(|e| Error::Other(format!("AVAssetReader: {}", e.localizedDescription())))?;
        let output = unsafe {
            AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
                &track,
                Some(&settings),
            )
        };
        unsafe { reader.addOutput(&output) };
        if !unsafe { reader.startReading() } {
            return Ok(false);
        }

        let mut wav = WavWriter::create(dest, 1, 16_000)?;
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(sb) = unsafe { output.copyNextSampleBuffer() } {
            let sb_ptr = &*sb as *const _ as *const c_void;
            let block = unsafe { CMSampleBufferGetDataBuffer(sb_ptr) };
            if block.is_null() {
                continue;
            }
            let len = unsafe { CMBlockBufferGetDataLength(block) };
            bytes.resize(len, 0);
            let status = unsafe {
                CMBlockBufferCopyDataBytes(block, 0, len, bytes.as_mut_ptr() as *mut c_void)
            };
            if status != 0 {
                return Err(Error::Other(format!(
                    "CMBlockBufferCopyDataBytes failed ({status})"
                )));
            }
            let samples: Vec<i16> = bytes
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            wav.write_i16(&samples)?;
        }
        wav.finalize()?;
        Ok(true)
    }
}
