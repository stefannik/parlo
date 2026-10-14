//! Audio pipeline: decode, resample, WAV I/O, encoding. Everything streams with bounded buffers.
pub mod decode;
pub mod encode;
pub mod mp4_edit;
pub mod prepare;
pub mod resample;
pub mod wav;
