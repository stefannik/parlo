//! Real microphone capture through cpal (CoreAudio on macOS, ALSA on Linux).
//!
//! STATUS: compiles on Linux; macOS behaviour (TCC permission, AirPods, route changes, sample-rate changes) is
//! DRAFTED, UNCHECKED — see docs/MAC_HANDOFF.md #4. The data callback only converts to f32 on a small stack buffer and
//! copies into the lock-free ring (`InputCtl::push`): no allocation, no locks, no I/O.

use super::audio_input::*;
use crate::error::{Error, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, SampleFormat};

pub struct CpalInput;

impl CpalInput {
    pub fn new() -> Self {
        Self
    }
}
impl Default for CpalInput {
    fn default() -> Self {
        Self::new()
    }
}

fn dev_id(d: &cpal::Device) -> String {
    d.id().map(|i| i.to_string()).unwrap_or_default()
}
fn dev_name(d: &cpal::Device) -> String {
    d.description()
        .map(|x| x.name().to_string())
        .unwrap_or_else(|_| dev_id(d))
}

fn map_err(e: cpal::Error) -> Error {
    match e.kind() {
        ErrorKind::PermissionDenied => Error::MicPermission,
        _ => Error::Device(e.to_string()),
    }
}

struct StreamGuard(#[allow(dead_code)] cpal::Stream);
// cpal::Stream is !Send on some backends (CoreAudio holds raw pointers); the guard is only created and dropped on the
// supervisor thread that opened it, and moved nowhere else.
unsafe impl Send for StreamGuard {}

impl AudioInput for CpalInput {
    fn devices(&self) -> Result<Vec<DeviceInfo>> {
        let host = cpal::default_host();
        let def = host.default_input_device().map(|d| dev_id(&d));
        let mut v = Vec::new();
        for d in host.input_devices().map_err(map_err)? {
            let id = dev_id(&d);
            v.push(DeviceInfo {
                is_default: Some(&id) == def.as_ref(),
                name: dev_name(&d),
                id,
            });
        }
        Ok(v)
    }

    fn default_device_id(&self) -> Option<String> {
        cpal::default_host()
            .default_input_device()
            .map(|d| dev_id(&d))
    }

    fn open(&self, sel: &DeviceSelector, mut ctl: InputCtl) -> Result<OpenedStream> {
        let host = cpal::default_host();
        let device = match sel {
            DeviceSelector::SystemDefault => host
                .default_input_device()
                .ok_or_else(|| Error::Device("no input device available".into()))?,
            DeviceSelector::Named(id) => {
                let parsed = id
                    .parse()
                    .map_err(|_| Error::Device(format!("bad device id '{id}'")))?;
                host.device_by_id(&parsed)
                    .ok_or_else(|| Error::Device(format!("device '{id}' is not available")))?
            }
        };
        let config = device.default_input_config().map_err(map_err)?;
        let (rate, channels, fmt) = (
            config.sample_rate() as u32,
            config.channels(),
            config.sample_format(),
        );
        let events = ctl.events.clone();
        let err_fn = move |err: cpal::Error| match err.kind() {
            // route changed but the stream keeps running (cpal follows the default device): nothing to rebuild
            ErrorKind::DeviceChanged | ErrorKind::Xrun => {
                tracing::warn!("input stream notice: {err}")
            }
            _ => {
                let _ = events.send(InputEvent::StreamError(err.to_string()));
            }
        };
        macro_rules! build {
            ($t:ty, $conv:expr) => {{
                let mut scratch = [0f32; 4096];
                device.build_input_stream(
                    config.clone().into(),
                    move |data: &[$t], _: &_| {
                        for chunk in data.chunks(scratch.len()) {
                            for (d, s) in scratch.iter_mut().zip(chunk) {
                                *d = $conv(*s);
                            }
                            ctl.push(&scratch[..chunk.len()]);
                        }
                    },
                    err_fn,
                    None,
                )
            }};
        }
        let stream = match fmt {
            SampleFormat::F32 => build!(f32, |s: f32| s),
            SampleFormat::I16 => build!(i16, |s: i16| s as f32 / 32768.0),
            SampleFormat::I32 => build!(i32, |s: i32| s as f32 / 2_147_483_648.0),
            SampleFormat::U16 => build!(u16, |s: u16| (s as f32 - 32768.0) / 32768.0),
            other => return Err(Error::Device(format!("unsupported sample format {other}"))),
        }
        .map_err(map_err)?;
        stream.play().map_err(map_err)?;
        Ok(OpenedStream {
            sample_rate: rate,
            channels,
            device_name: dev_name(&device),
            guard: Box::new(StreamGuard(stream)),
        })
    }
}
