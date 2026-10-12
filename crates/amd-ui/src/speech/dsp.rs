//! The sound side of speech to text (ticket 187): bringing the microphone's
//! rate to Whisper's 16 kHz, and Whisper's 80 mel filters. Our own small
//! code, rather than a resampling or DSP crate.

/// Whisper listens at 16 kHz.
pub const WHISPER_RATE: u32 = 16_000;

/// `samples` at `from` Hz, resampled to `to` Hz with a windowed-sinc filter
/// (cut off just below the lower Nyquist rate, so nothing folds back).
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = f64::from(to) / f64::from(from);
    // Lowpass at the narrower of the two bands, a little under Nyquist.
    let cutoff = ratio.min(1.0) * 0.94;
    const HALF: f64 = 16.0;
    let reach = (HALF / cutoff).ceil() as i64;
    let out_len = ((samples.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len {
        let t = n as f64 / ratio;
        let centre = t.floor() as i64;
        let mut sum = 0.0;
        let mut weight = 0.0;
        for k in (centre - reach)..=(centre + reach) {
            if k < 0 || k as usize >= samples.len() {
                continue;
            }
            let x = (t - k as f64) * cutoff;
            if x.abs() > HALF {
                continue;
            }
            let sinc = if x == 0.0 {
                1.0
            } else {
                (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
            };
            // A Hann window over the filter's reach.
            let window = 0.5 * (1.0 + (std::f64::consts::PI * x / HALF).cos());
            let w = sinc * window;
            sum += f64::from(samples[k as usize]) * w;
            weight += w;
        }
        out.push(if weight.abs() > 1e-9 {
            (sum / weight) as f32
        } else {
            0.0
        });
    }
    out
}

/// `samples` (16 kHz) without the burst a microphone makes as it wakes up:
/// PipeWire suspends an idle input, and its first fraction of a second can
/// be a loud pop (RMS near 1.0 measured here) that Whisper then "hears" as
/// a word. Anything in the first half-second louder than speech gets near
/// a microphone (RMS 0.3 over 10 ms) is cut, with 20 ms more for its tail.
pub fn trim_wake_up(samples: &[f32]) -> &[f32] {
    const WINDOW: usize = (WHISPER_RATE / 100) as usize; // 10 ms
    let search = samples.len().min(WHISPER_RATE as usize / 2);
    let mut cut = 0;
    let mut start = 0;
    while start + WINDOW <= search {
        let chunk = &samples[start..start + WINDOW];
        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / WINDOW as f32).sqrt();
        if rms > 0.3 {
            cut = start + WINDOW;
        }
        start += WINDOW;
    }
    if cut > 0 {
        cut = (cut + 2 * WINDOW).min(samples.len());
    }
    &samples[cut..]
}

/// Whisper's mel filterbank: `n_mels` triangular filters over the
/// `n_fft / 2 + 1` frequency bins of a 16 kHz signal, on the Slaney mel
/// scale with Slaney normalisation (librosa's defaults, which Whisper's
/// own `mel_filters.npz` was made with). Row-major, `n_mels` rows.
pub fn mel_filters(n_mels: usize, n_fft: usize) -> Vec<f32> {
    let sr = f64::from(WHISPER_RATE);
    let bins = n_fft / 2 + 1;
    let fft_freqs: Vec<f64> = (0..bins).map(|i| i as f64 * sr / n_fft as f64).collect();
    let (mel_min, mel_max) = (hz_to_mel(0.0), hz_to_mel(sr / 2.0));
    let mel_f: Vec<f64> = (0..n_mels + 2)
        .map(|i| mel_to_hz(mel_min + (mel_max - mel_min) * i as f64 / (n_mels + 1) as f64))
        .collect();
    let mut weights = vec![0f32; n_mels * bins];
    for m in 0..n_mels {
        let (lo, mid, hi) = (mel_f[m], mel_f[m + 1], mel_f[m + 2]);
        let norm = 2.0 / (hi - lo);
        for (b, f) in fft_freqs.iter().enumerate() {
            let rising = (f - lo) / (mid - lo);
            let falling = (hi - f) / (hi - mid);
            let w = rising.min(falling).max(0.0);
            weights[m * bins + b] = (w * norm) as f32;
        }
    }
    weights
}

/// The Slaney mel scale: linear to 1 kHz, logarithmic above.
fn hz_to_mel(hz: f64) -> f64 {
    const F_SP: f64 = 200.0 / 3.0;
    const MIN_LOG_HZ: f64 = 1000.0;
    let min_log_mel = MIN_LOG_HZ / F_SP;
    let logstep = 6.4f64.ln() / 27.0;
    if hz >= MIN_LOG_HZ {
        min_log_mel + (hz / MIN_LOG_HZ).ln() / logstep
    } else {
        hz / F_SP
    }
}

fn mel_to_hz(mel: f64) -> f64 {
    const F_SP: f64 = 200.0 / 3.0;
    const MIN_LOG_HZ: f64 = 1000.0;
    let min_log_mel = MIN_LOG_HZ / F_SP;
    let logstep = 6.4f64.ln() / 27.0;
    if mel >= min_log_mel {
        MIN_LOG_HZ * (logstep * (mel - min_log_mel)).exp()
    } else {
        F_SP * mel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, rate: u32, seconds: f64) -> Vec<f32> {
        let n = (f64::from(rate) * seconds) as usize;
        (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(rate)).sin() as f32)
            .collect()
    }

    /// How many times the signal crosses zero going up: its frequency × time.
    fn rising_crossings(s: &[f32]) -> usize {
        s.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    #[test]
    fn resampling_keeps_length_and_pitch() {
        let input = tone(440.0, 48_000, 1.0);
        let out = resample(&input, 48_000, WHISPER_RATE);
        assert_eq!(out.len(), 16_000);
        let crossings = rising_crossings(&out);
        assert!((438..=442).contains(&crossings), "{crossings}");
        let peak = out[2000..14000].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!((0.9..1.1).contains(&peak), "{peak}");
        // Upsampling too.
        assert_eq!(
            resample(&tone(440.0, 8_000, 0.5), 8_000, WHISPER_RATE).len(),
            8_000
        );
        assert_eq!(resample(&[], 48_000, 16_000), Vec::<f32>::new());
    }

    #[test]
    fn resampling_removes_what_cannot_be_kept() {
        // 12 kHz can't exist at 16 kHz; it must not come out as 4 kHz.
        let out = resample(&tone(12_000.0, 48_000, 0.5), 48_000, WHISPER_RATE);
        let peak = out[1000..7000].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak < 0.1, "{peak}");
    }

    #[test]
    fn the_wake_up_pop_is_cut_and_speech_kept() {
        let mut pcm = vec![0.9f32; 4000]; // a quarter-second pop
        pcm.extend(tone(300.0, WHISPER_RATE, 1.0).iter().map(|s| s * 0.1));
        let kept = trim_wake_up(&pcm);
        assert_eq!(kept.len(), 16_000 - 320, "the pop and 20 ms after it go");
        let quiet = tone(300.0, WHISPER_RATE, 1.0)
            .iter()
            .map(|s| s * 0.1)
            .collect::<Vec<_>>();
        assert_eq!(
            trim_wake_up(&quiet).len(),
            quiet.len(),
            "no pop, nothing cut"
        );
        assert!(trim_wake_up(&[]).is_empty());
    }

    #[test]
    fn mel_filters_have_whispers_shape() {
        let f = mel_filters(80, 400);
        assert_eq!(f.len(), 80 * 201);
        for m in 0..80 {
            let row = &f[m * 201..(m + 1) * 201];
            assert!(row.iter().sum::<f32>() > 0.0, "filter {m} is empty");
            assert!(row.iter().all(|w| *w >= 0.0));
        }
        // The first filter peaks in the lowest bins; the last near 8 kHz.
        let argmax = |m: usize| {
            let row = &f[m * 201..(m + 1) * 201];
            row.iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .unwrap()
                .0
        };
        assert!(argmax(0) <= 2);
        assert!(argmax(79) >= 180);
        assert!((hz_to_mel(mel_to_hz(25.0)) - 25.0).abs() < 1e-9);
    }
}
