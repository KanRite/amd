//! Recording from the microphone with `cpal` (ticket 187). A stream lives
//! on a thread of its own (it isn't `Send` everywhere), collecting mono
//! samples at the device's own rate until told to stop; the recording then
//! comes back resampled to Whisper's 16 kHz, at most 30 seconds of it.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample, StreamConfig};

use super::dsp::{WHISPER_RATE, resample, trim_wake_up};

/// Whisper hears at most 30 seconds at a time.
const MAX_SECONDS: usize = 30;

/// The default microphone's name, if there is one.
pub fn microphone() -> Option<String> {
    let device = cpal::default_host().default_input_device()?;
    device
        .description()
        .ok()
        .map(|d| d.name().to_string())
        .or_else(|| Some("the default microphone".to_string()))
}

/// A recording in progress.
pub struct Recorder {
    stop: mpsc::Sender<()>,
    done: mpsc::Receiver<Result<(Vec<f32>, u32)>>,
}

impl Recorder {
    /// Starts recording from the default microphone.
    pub fn start() -> Result<Recorder> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (done_tx, done_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        std::thread::spawn(move || {
            let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
            let opened = open(Arc::clone(&samples));
            let (stream, rate) = match opened {
                Ok(pair) => {
                    let _ = ready_tx.send(Ok(()));
                    pair
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            // Until stopped (or the recorder is dropped).
            let _ = stop_rx.recv();
            drop(stream);
            let recorded = std::mem::take(&mut *samples.lock().unwrap_or_else(|e| e.into_inner()));
            let _ = done_tx.send(Ok((recorded, rate)));
        });
        ready_rx
            .recv()
            .map_err(|_| anyhow!("the recording thread stopped"))??;
        Ok(Recorder {
            stop: stop_tx,
            done: done_rx,
        })
    }

    /// Stops and returns the recording at 16 kHz.
    pub fn stop(self) -> Result<Vec<f32>> {
        let _ = self.stop.send(());
        let (samples, rate) = self
            .done
            .recv()
            .map_err(|_| anyhow!("the recording thread stopped"))??;
        let pcm = resample(&samples, rate, WHISPER_RATE);
        Ok(trim_wake_up(&pcm).to_vec())
    }
}

/// Opens the default input and starts it, mixing every frame down to one
/// channel into `samples`.
fn open(samples: Arc<Mutex<Vec<f32>>>) -> Result<(cpal::Stream, u32)> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or_else(|| anyhow!("no microphone found"))?;
    let supported = device
        .default_input_config()
        .context("asking the microphone how it records")?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let rate = config.sample_rate;
    let channels = usize::from(config.channels.max(1));
    let cap = MAX_SECONDS * rate as usize;
    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, channels, cap, samples)?,
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, channels, cap, samples)?,
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, channels, cap, samples)?,
        cpal::SampleFormat::I32 => build::<i32>(&device, &config, channels, cap, samples)?,
        other => {
            return Err(anyhow!(
                "the microphone records as {other:?}, which amdui can't read"
            ));
        }
    };
    stream.play().context("starting the microphone")?;
    Ok((stream, rate))
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    cap: usize,
    samples: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream>
where
    T: SizedSample + Sample,
    f32: FromSample<T>,
{
    let stream = device
        .build_input_stream::<T, _, _>(
            *config,
            move |data: &[T], _| {
                let Ok(mut out) = samples.lock() else { return };
                for frame in data.chunks(channels) {
                    if out.len() >= cap {
                        return;
                    }
                    let sum: f32 = frame.iter().map(|s| f32::from_sample(*s)).sum();
                    out.push(sum / frame.len() as f32);
                }
            },
            |e| eprintln!("amdui: microphone: {e}"),
            None,
        )
        .context("opening the microphone")?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records two seconds while ALSA's "Front center" clip plays, and
    /// prints how loud it was. Needs a microphone and speakers.
    #[test]
    #[ignore]
    fn records_what_the_speakers_play() {
        let recorder = Recorder::start().unwrap();
        let _ = std::process::Command::new("pw-play")
            .arg("/usr/share/sounds/alsa/Front_Center.wav")
            .status();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let pcm = recorder.stop().unwrap();
        let rms = (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len().max(1) as f32).sqrt();
        let peak = pcm.iter().fold(0f32, |m, s| m.max(s.abs()));
        eprintln!(
            "{} samples at 16 kHz, rms {rms:.4}, peak {peak:.4}",
            pcm.len()
        );
        if crate::speech::model::Model::TinyEn.installed() {
            let mut engine =
                crate::speech::whisper::Engine::load(crate::speech::model::Model::TinyEn).unwrap();
            eprintln!("heard: {:?}", engine.transcribe(&pcm).unwrap());
        }
    }
}
