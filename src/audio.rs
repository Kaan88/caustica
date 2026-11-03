//! Track 19 audio-reactive lights: WASAPI loopback capture of what the PC is playing (cpal
//! builds an input stream on the default output device), an FFT of the latest samples into
//! bass, mid and treble levels, and those levels scaling the lights and emitters of the frame
//! being rendered. The scene itself is never changed: `Audio::apply` works on the per-frame
//! copy, so saving, undo and editing see the user's values.
use crate::scene::{AudioReact, Settings};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Samples analysed per frame (about 43 ms at 48 kHz).
const WINDOW: usize = 2048;
/// Band edges in Hz: bass, mid, treble.
const BANDS: [(f32, f32); 3] = [(20., 250.), (250., 2000.), (2000., 12000.)];

/// Energy per band of `samples` (mono) at `rate`, as the RMS magnitude of its FFT bins
/// (Hann window).
#[cfg(test)]
fn bands(samples: &[f32], rate: f32) -> [f32; 3] {
    bands_with(&mut rustfft::FftPlanner::new(), samples, rate)
}
/// `bands` with a planner that keeps its FFT plans between calls.
fn bands_with(planner: &mut rustfft::FftPlanner<f32>, samples: &[f32], rate: f32) -> [f32; 3] {
    let n = samples.len();
    if n < 64 {
        return [0.; 3];
    }
    let mut buf: Vec<rustfft::num_complex::Complex<f32>> = samples
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (n - 1) as f32).cos();
            rustfft::num_complex::Complex::new(x * w, 0.)
        })
        .collect();
    planner.plan_fft_forward(n).process(&mut buf);
    let hz = rate / n as f32;
    BANDS.map(|(lo, hi)| {
        let a = ((lo / hz).ceil() as usize).max(1);
        let b = ((hi / hz) as usize).min(n / 2);
        if b <= a {
            return 0.;
        }
        let sum: f32 = buf[a..b].iter().map(|c| c.norm_sqr()).sum();
        (sum / (b - a) as f32).sqrt() / n as f32
    })
}

/// The running capture and the smoothed, normalised band levels.
pub struct Audio {
    _stream: cpal::Stream,
    /// The latest samples (mono) and when they arrived: loopback sends nothing during
    /// silence, so an old window counts as silence.
    ring: Arc<Mutex<(VecDeque<f32>, std::time::Instant)>>,
    rate: f32,
    /// Smoothed levels, 0..1 against each band's recent peak.
    pub levels: [f32; 3],
    /// Beat envelope (track 19): jumps to 1 on a bass onset, then decays (half-life 80 ms).
    pub beat: f32,
    /// Running mean of the raw bass energy and the time since the last beat.
    bass_mean: f32,
    since_beat: f32,
    /// Each band's slowly decaying peak (automatic gain).
    peaks: [f32; 3],
    planner: rustfft::FftPlanner<f32>,
    /// The stream reported an error. cpal also reports a change of the default output
    /// device this way (the stream keeps capturing the old one), and some drivers and audio
    /// tools send that notification as soon as a capture opens, so it is not fatal: `recover`
    /// reopens the capture on the current default device.
    error: Arc<std::sync::atomic::AtomicBool>,
    started: std::time::Instant,
}

impl Audio {
    /// Starts loopback capture of the default output device.
    pub fn start() -> anyhow::Result<Self> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| anyhow::anyhow!("no audio output device"))?;
        let config = device.default_output_config()?;
        anyhow::ensure!(
            config.sample_format() == cpal::SampleFormat::F32,
            "audio output is {:?}, not f32",
            config.sample_format()
        );
        let channels = config.channels().max(1) as usize;
        let rate = config.sample_rate() as f32;
        let ring = Arc::new(Mutex::new((
            VecDeque::with_capacity(WINDOW * 2),
            std::time::Instant::now(),
        )));
        let sink = ring.clone();
        let error = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = error.clone();
        let stream = device.build_input_stream(
            config.config(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if let Ok(mut guard) = sink.lock() {
                    let (r, at) = &mut *guard;
                    *at = std::time::Instant::now();
                    for frame in data.chunks(channels) {
                        r.push_back(frame.iter().sum::<f32>() / channels as f32);
                    }
                    let excess = r.len().saturating_sub(WINDOW);
                    r.drain(..excess);
                }
            },
            move |e| {
                eprintln!("audio: {e}");
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            },
            None,
        )?;
        stream.play()?;
        Ok(Self {
            _stream: stream,
            ring,
            rate,
            levels: [0.; 3],
            beat: 0.,
            bass_mean: -1.,
            since_beat: 1.,
            peaks: [1e-4; 3],
            planner: rustfft::FftPlanner::new(),
            error,
            started: std::time::Instant::now(),
        })
    }
    /// After a stream error, reopens the capture on the current default output device (at
    /// most once a second, so a device that keeps signalling costs one reopen a second), and
    /// keeps the levels and gains. An error only when the device cannot be opened.
    pub fn recover(&mut self) -> anyhow::Result<()> {
        if !self.error.load(std::sync::atomic::Ordering::Relaxed)
            || self.started.elapsed() < std::time::Duration::from_secs(1)
        {
            return Ok(());
        }
        let mut next = Self::start()?;
        next.levels = self.levels;
        next.peaks = self.peaks;
        next.planner = std::mem::replace(&mut self.planner, rustfft::FftPlanner::new());
        *self = next;
        Ok(())
    }
    /// Analyses the latest window: call once per frame with the frame time.
    pub fn update(&mut self, a: &AudioReact, dt: f32) {
        let samples: Vec<f32> = self
            .ring
            .lock()
            .map(|r| {
                if r.1.elapsed().as_millis() < 200 {
                    r.0.iter().copied().collect()
                } else {
                    vec![]
                }
            })
            .unwrap_or_default();
        let raw = bands_with(&mut self.planner, &samples, self.rate);
        // Smoothing is per second of frame time, so it feels the same at any frame rate.
        let keep = a.smoothing.clamp(0., 0.99).powf(dt * 60.);
        let decay = 0.5f32.powf(dt / 4.); // peaks halve in 4 s
        for ((peak, level), raw) in self.peaks.iter_mut().zip(&mut self.levels).zip(raw) {
            *peak = (*peak * decay).max(raw).max(1e-4);
            *level = *level * keep + (raw / *peak).clamp(0., 1.) * (1. - keep);
        }
        (self.beat, self.bass_mean, self.since_beat, _) = beat_step(
            self.beat,
            self.bass_mean,
            self.since_beat,
            raw[0],
            self.peaks[0],
            dt,
        );
    }
}

/// One step of the beat detector: an onset is bass energy 1.5× its running mean (about a
/// second long), above 30% of the band's peak, at least 0.25 s after the last one. Returns the
/// new envelope, mean, time since the last beat, and whether this step was an onset.
fn beat_step(
    beat: f32,
    mean: f32,
    since: f32,
    bass: f32,
    peak: f32,
    dt: f32,
) -> (f32, f32, f32, bool) {
    // A negative mean is "no history yet": the first window only starts it.
    if mean < 0. {
        return (beat, bass, since + dt, false);
    }
    let onset = bass > 1.5 * mean && bass > 0.3 * peak && since > 0.25;
    let mean = mean + (bass - mean) * (1. - 0.5f32.powf(dt / 0.7));
    let beat = if onset {
        1.
    } else {
        beat * 0.5f32.powf(dt / 0.08)
    };
    (beat, mean, if onset { 0. } else { since + dt }, onset)
}
/// Scales the lights and emitters of `s` by the band levels: the main light follows the bass,
/// area light i band i % 3, emissive object i band i % 3. A level of 0 gives `floor` of the
/// set intensity, a level of 1 gives `floor + gain`.
/// `emitters`: also scale emissive objects.
pub fn apply(s: &mut Settings, levels: [f32; 3], beat: f32, emitters: bool) {
    let a = s.audio;
    let scale = |band: usize| a.floor + a.gain * levels[band % 3];
    s.light_intensity *= scale(0);
    const BAND_COLORS: [[f32; 3]; 4] = [[1., 0.25, 0.1], [0.3, 1., 0.3], [0.3, 0.5, 1.], [1.; 3]];
    for (i, l) in s.lights.iter_mut().enumerate() {
        // The light's band: 0..3 bass, mid, treble, 3 beats, None for nothing.
        let band = match l.audio_band {
            0 => Some(i % 3),
            b @ 1..=4 => Some(b as usize - 1),
            _ => None,
        };
        let Some(band) = band else { continue };
        let level = if band == 3 { beat } else { levels[band] };
        l.intensity = (l.intensity * (a.floor + a.gain * level)).min(2000.);
        let w = a.color_shift * level;
        for (c, b) in l.color.iter_mut().zip(BAND_COLORS[band]) {
            *c = (*c * (1. - w) + b * w).clamp(0., 1.);
        }
    }
    for (i, o) in s.objects.iter_mut().enumerate() {
        if emitters && o.material.emission > 0. {
            o.material.emission *= scale(i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_lands_in_its_band() {
        let tone = |hz: f32| -> Vec<f32> {
            (0..WINDOW)
                .map(|i| (std::f32::consts::TAU * hz * i as f32 / 48000.).sin())
                .collect()
        };
        for (hz, band) in [(80., 0), (900., 1), (5000., 2)] {
            let b = bands(&tone(hz), 48000.);
            let loudest = (0..3).max_by(|&x, &y| b[x].total_cmp(&b[y])).unwrap();
            assert_eq!(loudest, band, "{hz} Hz: {b:?}");
        }
        assert_eq!(bands(&vec![0.; WINDOW], 48000.), [0.; 3]);
    }

    /// Needs an audio output device: `cargo test -- --ignored loopback`.
    #[test]
    #[ignore]
    fn loopback_capture_opens_and_delivers_samples() {
        let a = Audio::start().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let n = a.ring.lock().unwrap().0.len();
        println!("rate {} samples {n}", a.rate);
    }

    #[test]
    fn apply_scales_lights_by_their_band() {
        let mut s = Settings {
            audio: AudioReact {
                enabled: true,
                gain: 2.,
                floor: 0.5,
                smoothing: 0.5,
                color_shift: 0.,
            },
            ..Settings::default()
        };
        let base = s.light_intensity;
        apply(&mut s, [1., 0., 0.], 0., true);
        assert!((s.light_intensity - base * 2.5).abs() < 1e-4);
        // A light on beats with full colour shift turns white at a beat; one set to nothing
        // keeps its intensity.
        s.audio.color_shift = 1.;
        s.lights = vec![
            crate::scene::AreaLight {
                audio_band: 4,
                color: [1., 0., 0.],
                ..Default::default()
            },
            crate::scene::AreaLight {
                audio_band: 5,
                ..Default::default()
            },
        ];
        let before = s.lights[1].intensity;
        apply(&mut s, [0.; 3], 1., true);
        assert_eq!(s.lights[0].color, [1.; 3]);
        assert_eq!(s.lights[1].intensity, before);
    }
    #[test]
    fn beats_fire_on_bass_onsets_only() {
        let (mut beat, mut mean, mut since) = (0., 0.1, 1.);
        let mut onsets = 0;
        // Steady bass for 2 s, then a kick every 0.5 s.
        for f in 0..240 {
            let t = f as f32 / 60.;
            let kick = t > 2. && (t % 0.5) < 1. / 60.;
            let bass = if kick { 1. } else { 0.1 };
            let o;
            (beat, mean, since, o) = beat_step(beat, mean, since, bass, 1., 1. / 60.);
            onsets += o as u32;
        }
        assert_eq!(onsets, 4);
        assert!(beat < 1.);
    }
}
