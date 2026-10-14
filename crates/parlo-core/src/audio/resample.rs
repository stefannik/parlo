//! Streaming mono resampler (rubato, synchronous FFT). Bounded memory; arbitrary-size input blocks.

use crate::error::{Error, Result};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

const CHUNK: usize = 1024;

pub struct StreamResampler {
    inner: Option<Fft<f32>>, // None => pass-through (rates equal)
    in_rate: u32,
    out_rate: u32,
    pending: Vec<f32>,
    out_chunk: Vec<f32>,
    total_in: u64,
    total_out: u64,
    skip: usize, // resampler delay still to discard
}

impl StreamResampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Result<Self> {
        if in_rate == out_rate {
            return Ok(Self { inner: None, in_rate, out_rate, pending: vec![], out_chunk: vec![], total_in: 0, total_out: 0, skip: 0 });
        }
        let r = Fft::<f32>::new(in_rate as usize, out_rate as usize, CHUNK, 1, FixedSync::Input).map_err(|e| Error::Other(format!("resampler: {e}")))?;
        let skip = r.output_delay();
        let out_chunk = vec![0f32; r.output_frames_max()];
        Ok(Self { inner: Some(r), in_rate, out_rate, pending: Vec::with_capacity(CHUNK * 2), out_chunk, total_in: 0, total_out: 0, skip })
    }

    fn run_chunk(&mut self, partial: Option<usize>, out: &mut Vec<f32>) -> Result<()> {
        let r = self.inner.as_mut().unwrap();
        let need = r.input_frames_next();
        let mut buf = vec![0f32; need];
        let n = partial.unwrap_or(need).min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        let inp = InterleavedSlice::new(&buf, 1, need).map_err(|e| Error::Other(format!("resampler: {e}")))?;
        let max_out = self.out_chunk.len();
        let mut outb = InterleavedSlice::new_mut(&mut self.out_chunk, 1, max_out).map_err(|e| Error::Other(format!("resampler: {e}")))?;
        let idx = rubato::Indexing::new().partial_len(n);
        let (_consumed, produced) = r.process_into_buffer(&inp, &mut outb, Some(&idx)).map_err(|e| Error::Other(format!("resampler: {e}")))?;
        self.pending.drain(..n);
        let mut s = &self.out_chunk[..produced];
        if self.skip > 0 {
            let k = self.skip.min(s.len());
            s = &s[k..];
            self.skip -= k;
        }
        out.extend_from_slice(s);
        Ok(())
    }

    /// Feed input samples; appends resampled output to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<()> {
        self.total_in += input.len() as u64;
        if self.inner.is_none() {
            out.extend_from_slice(input);
            self.total_out += input.len() as u64;
            return Ok(());
        }
        self.pending.extend_from_slice(input);
        let before = out.len();
        while self.pending.len() >= self.inner.as_ref().unwrap().input_frames_next() {
            self.run_chunk(None, out)?;
        }
        self.total_out += (out.len() - before) as u64;
        Ok(())
    }

    /// Flush the tail so that exactly `ceil(total_in * out/in)` samples have been produced overall.
    pub fn finish(&mut self, out: &mut Vec<f32>) -> Result<()> {
        if self.inner.is_none() {
            return Ok(());
        }
        let expected = (self.total_in as u128 * self.out_rate as u128).div_ceil(self.in_rate as u128) as u64;
        let before = out.len();
        // remaining real input (partial chunk), then silence to flush the filter delay
        let mut guard = 0;
        let mut first = true;
        while self.total_out + ((out.len() - before) as u64) < expected && guard < 16 {
            let part = if first { Some(self.pending.len()) } else { Some(0) };
            first = false;
            self.run_chunk(part, out)?;
            guard += 1;
        }
        let produced = (out.len() - before) as u64;
        let total = self.total_out + produced;
        if total > expected {
            let cut = (total - expected) as usize;
            out.truncate(out.len() - cut);
        }
        self.total_out = expected;
        Ok(())
    }
}
