use crate::core::{Snapshot, SoundPreview};
use crate::timer::{FinishSound, FocusSound, Mode};
use rodio::buffer::SamplesBuffer;
use rodio::source::noise::{Blue, Brownian, Pink, Violet, WhiteUniform};
use rodio::{ChannelCount, DeviceSinkBuilder, MixerDeviceSink, Player, Sample, SampleRate, Source};
use std::f32::consts::TAU;
use std::num::NonZero;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;

const FADE_DURATION: Duration = Duration::from_millis(250);
const NOISE_PREVIEW_DURATION: Duration = Duration::from_secs(3);
const FINISH_PREVIEW_DURATION: Duration = Duration::from_secs(1);
const PREVIEW_FADE_IN: Duration = Duration::from_millis(100);

pub fn spawn(updates: UnboundedReceiver<Snapshot>) {
    thread::Builder::new()
        .name("pomolet-audio".into())
        .spawn(move || AudioWorker::new().run(updates))
        .expect("failed to start audio subscriber");
}

struct AudioWorker {
    device: Option<MixerDeviceSink>,
    noise: Option<NoisePlayback>,
    preview: Option<FadingPlayback>,
    completion: Option<Player>,
    last_completion: Option<u64>,
    last_preview_seq: u64,
    last_device_attempt: Option<Instant>,
}

struct NoisePlayback {
    sound: FocusSound,
    playback: FadingPlayback,
}

struct FadingPlayback {
    player: Player,
    fade_out: Arc<AtomicBool>,
}

impl FadingPlayback {
    fn fade_out(self) {
        self.fade_out.store(true, Ordering::Relaxed);
        self.player.detach();
    }
}

/// Keeps a source alive until a requested sample-level fade-out completes.
struct FadeOnRequest<S> {
    input: S,
    requested: Arc<AtomicBool>,
    fade_samples: u64,
    remaining: Option<u64>,
}

struct PreviewClip<S> {
    input: S,
    remaining_samples: u64,
    fade_samples: u64,
    duration: Duration,
}

impl<S: Source> PreviewClip<S> {
    fn new(input: S, duration: Duration) -> Self {
        let remaining_samples = sample_count(&input, duration);
        let fade_samples = sample_count(&input, FADE_DURATION).min(remaining_samples);
        Self {
            input,
            remaining_samples,
            fade_samples,
            duration,
        }
    }
}

impl<S: Source> Iterator for PreviewClip<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_samples == 0 {
            return None;
        }
        let sample = self.input.next()?;
        let gain = if self.remaining_samples <= self.fade_samples {
            (self.remaining_samples - 1) as f32 / self.fade_samples as f32
        } else {
            1.0
        };
        self.remaining_samples -= 1;
        Some(sample * gain)
    }
}

impl<S: Source> Source for PreviewClip<S> {
    fn current_span_len(&self) -> Option<usize> {
        if self.remaining_samples == 0 {
            Some(0)
        } else {
            None
        }
    }

    fn channels(&self) -> ChannelCount {
        self.input.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(self.duration)
    }
}

fn sample_count<S: Source>(source: &S, duration: Duration) -> u64 {
    (duration.as_secs_f64()
        * f64::from(source.sample_rate().get())
        * f64::from(source.channels().get()))
    .round()
    .max(1.0) as u64
}

impl<S: Source> FadeOnRequest<S> {
    fn new(input: S, duration: Duration) -> (Self, Arc<AtomicBool>) {
        let fade_samples = sample_count(&input, duration);
        let requested = Arc::new(AtomicBool::new(false));
        (
            Self {
                input,
                requested: requested.clone(),
                fade_samples,
                remaining: None,
            },
            requested,
        )
    }
}

impl<S: Source> Iterator for FadeOnRequest<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_none() && self.requested.load(Ordering::Relaxed) {
            self.remaining = Some(self.fade_samples);
        }
        let gain = if let Some(remaining) = &mut self.remaining {
            if *remaining == 0 {
                return None;
            }
            *remaining -= 1;
            *remaining as f32 / self.fade_samples as f32
        } else {
            1.0
        };
        self.input.next().map(|sample| sample * gain)
    }
}

impl<S: Source> Source for FadeOnRequest<S> {
    fn current_span_len(&self) -> Option<usize> {
        if self.remaining == Some(0) {
            Some(0)
        } else {
            self.input.current_span_len()
        }
    }

    fn channels(&self) -> ChannelCount {
        self.input.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }
}

fn append_fading<S: Source + Send + 'static>(player: &Player, source: S) -> Arc<AtomicBool> {
    let (source, fade_out) = FadeOnRequest::new(source, FADE_DURATION);
    player.append(source);
    fade_out
}

impl AudioWorker {
    fn new() -> Self {
        Self {
            device: None,
            noise: None,
            preview: None,
            completion: None,
            last_completion: None,
            last_preview_seq: 0,
            last_device_attempt: None,
        }
    }

    fn run(mut self, mut updates: UnboundedReceiver<Snapshot>) {
        while let Some(snapshot) = updates.blocking_recv() {
            self.apply(snapshot);
        }
    }

    fn apply(&mut self, snapshot: Snapshot) {
        let completed = self
            .last_completion
            .replace(snapshot.completed_phases)
            .is_some_and(|previous| previous != snapshot.completed_phases);

        let wanted_noise = wanted_noise(&snapshot);
        let active_noise = self.noise.as_ref().map(|noise| noise.sound);
        let wanted_active = (wanted_noise != FocusSound::Off).then_some(wanted_noise);
        if active_noise != wanted_active {
            if let Some(noise) = self.noise.take() {
                noise.playback.fade_out();
            }
            if let Some(sound) = wanted_active {
                self.start_noise(sound);
            }
        }

        if completed && snapshot.settings.finish_sound != FinishSound::Off {
            self.play_completion(snapshot.settings.finish_sound);
        }

        if snapshot.sound_preview_seq != self.last_preview_seq {
            self.last_preview_seq = snapshot.sound_preview_seq;
            if let Some(preview) = self.preview.take() {
                preview.fade_out();
            }
            if let Some(sound) = snapshot.sound_preview {
                self.play_preview(sound);
            }
        }
    }

    fn start_noise(&mut self, sound: FocusSound) {
        let Some(device) = self.device() else {
            return;
        };
        let sample_rate = device.config().sample_rate();
        let player = Player::connect_new(device.mixer());
        let fade_out = match sound {
            FocusSound::Off => return,
            FocusSound::Brown => append_fading(
                &player,
                Brownian::new(sample_rate)
                    .amplify(0.08)
                    .fade_in(FADE_DURATION),
            ),
            FocusSound::Pink => append_fading(
                &player,
                Pink::new(sample_rate).amplify(0.12).fade_in(FADE_DURATION),
            ),
            FocusSound::White => append_fading(
                &player,
                WhiteUniform::new(sample_rate)
                    .amplify(0.07)
                    .fade_in(FADE_DURATION),
            ),
            FocusSound::Blue => append_fading(
                &player,
                Blue::new(sample_rate).amplify(0.05).fade_in(FADE_DURATION),
            ),
            FocusSound::Violet => append_fading(
                &player,
                Violet::new(sample_rate)
                    .amplify(0.025)
                    .fade_in(FADE_DURATION),
            ),
        };
        self.noise = Some(NoisePlayback {
            sound,
            playback: FadingPlayback { player, fade_out },
        });
    }

    fn play_preview(&mut self, sound: SoundPreview) {
        if matches!(
            sound,
            SoundPreview::Focus(FocusSound::Off) | SoundPreview::Finish(FinishSound::Off)
        ) || (matches!(sound, SoundPreview::Focus(_)) && self.noise.is_some())
        {
            return;
        }

        let Some(device) = self.device() else {
            return;
        };
        let sample_rate = device.config().sample_rate();
        let player = Player::connect_new(device.mixer());
        let fade_out = match sound {
            SoundPreview::Focus(FocusSound::Brown) => append_fading(
                &player,
                PreviewClip::new(
                    Brownian::new(sample_rate)
                        .amplify(0.08)
                        .fade_in(PREVIEW_FADE_IN),
                    NOISE_PREVIEW_DURATION,
                ),
            ),
            SoundPreview::Focus(FocusSound::Pink) => append_fading(
                &player,
                PreviewClip::new(
                    Pink::new(sample_rate)
                        .amplify(0.12)
                        .fade_in(PREVIEW_FADE_IN),
                    NOISE_PREVIEW_DURATION,
                ),
            ),
            SoundPreview::Focus(FocusSound::White) => append_fading(
                &player,
                PreviewClip::new(
                    WhiteUniform::new(sample_rate)
                        .amplify(0.07)
                        .fade_in(PREVIEW_FADE_IN),
                    NOISE_PREVIEW_DURATION,
                ),
            ),
            SoundPreview::Focus(FocusSound::Blue) => append_fading(
                &player,
                PreviewClip::new(
                    Blue::new(sample_rate)
                        .amplify(0.05)
                        .fade_in(PREVIEW_FADE_IN),
                    NOISE_PREVIEW_DURATION,
                ),
            ),
            SoundPreview::Focus(FocusSound::Violet) => append_fading(
                &player,
                PreviewClip::new(
                    Violet::new(sample_rate)
                        .amplify(0.025)
                        .fade_in(PREVIEW_FADE_IN),
                    NOISE_PREVIEW_DURATION,
                ),
            ),
            SoundPreview::Focus(FocusSound::Off) | SoundPreview::Finish(FinishSound::Off) => return,
            SoundPreview::Finish(finish) => append_fading(
                &player,
                PreviewClip::new(
                    completion_samples(finish, sample_rate),
                    FINISH_PREVIEW_DURATION,
                ),
            ),
        };
        self.preview = Some(FadingPlayback { player, fade_out });
    }

    fn play_completion(&mut self, sound: FinishSound) {
        let Some(device) = self.device() else {
            return;
        };
        let sample_rate = device.config().sample_rate();
        let player = Player::connect_new(device.mixer());
        player.append(completion_samples(sound, sample_rate));
        self.completion = Some(player);
    }

    fn device(&mut self) -> Option<&MixerDeviceSink> {
        if self.device.is_none() {
            if self
                .last_device_attempt
                .is_some_and(|attempt| attempt.elapsed() < Duration::from_secs(10))
            {
                return None;
            }
            self.last_device_attempt = Some(Instant::now());
            match DeviceSinkBuilder::open_default_sink() {
                Ok(mut device) => {
                    device.log_on_drop(false);
                    self.device = Some(device);
                }
                Err(error) => {
                    eprintln!("Audio output unavailable: {error}");
                    return None;
                }
            }
        }
        self.device.as_ref()
    }
}

fn wanted_noise(snapshot: &Snapshot) -> FocusSound {
    if snapshot.running && snapshot.mode == Mode::Focus {
        snapshot.settings.focus_sound
    } else {
        FocusSound::Off
    }
}

fn completion_samples(sound: FinishSound, sample_rate: SampleRate) -> SamplesBuffer {
    let seconds = match sound {
        FinishSound::Gong => 2.4,
        FinishSound::Chime => 1.4,
        FinishSound::Bell => 1.8,
        FinishSound::Wood => 0.8,
        FinishSound::Pulse => 1.1,
        FinishSound::Off => 0.0,
    };
    let rate = sample_rate.get();
    let count = (seconds * rate as f32) as usize;
    let samples = (0..count)
        .map(|index| {
            let time = index as f32 / rate as f32;
            match sound {
                FinishSound::Gong => {
                    let envelope = (time * 70.0).min(1.0) * (-1.8 * time).exp();
                    let fundamental = (TAU * 196.0 * time).sin() * 0.55;
                    let overtones = (TAU * 293.7 * time).sin() * 0.20
                        + (TAU * 415.3 * time).sin() * 0.12
                        + (TAU * 587.3 * time).sin() * 0.07;
                    (fundamental + overtones) * envelope * 0.32
                }
                FinishSound::Chime => {
                    let first = bell_note(time, 523.25);
                    let second = bell_note(time - 0.22, 783.99);
                    (first + second) * 0.25
                }
                FinishSound::Bell => {
                    let envelope = (time * 80.0).min(1.0) * (-2.8 * time).exp();
                    let tone = (TAU * 440.0 * time).sin() * 0.65
                        + (TAU * 660.0 * time).sin() * 0.25
                        + (TAU * 1050.0 * time).sin() * 0.10;
                    tone * envelope * 0.28
                }
                FinishSound::Wood => (wood_note(time) + wood_note(time - 0.27) * 0.7) * 0.32,
                FinishSound::Pulse => {
                    (pulse_note(time, 880.0)
                        + pulse_note(time - 0.22, 880.0)
                        + pulse_note(time - 0.44, 1174.66))
                        * 0.25
                }
                FinishSound::Off => 0.0,
            }
        })
        .collect::<Vec<_>>();
    SamplesBuffer::new(NonZero::new(1).unwrap(), sample_rate, samples)
}

fn bell_note(time: f32, frequency: f32) -> f32 {
    if time < 0.0 {
        return 0.0;
    }
    let envelope = (time * 100.0).min(1.0) * (-4.0 * time).exp();
    let tone = (TAU * frequency * time).sin() * 0.8 + (TAU * frequency * 2.01 * time).sin() * 0.18;
    tone * envelope
}

fn wood_note(time: f32) -> f32 {
    if time < 0.0 {
        return 0.0;
    }
    let envelope = (time * 350.0).min(1.0) * (-22.0 * time).exp();
    let tone = (TAU * 290.0 * time).sin() * 0.75 + (TAU * 680.0 * time).sin() * 0.25;
    tone * envelope
}

fn pulse_note(time: f32, frequency: f32) -> f32 {
    if time < 0.0 {
        return 0.0;
    }
    let envelope = (time * 200.0).min(1.0) * (-14.0 * time).exp();
    let tone = (TAU * frequency * time).sin() * 0.85 + (TAU * frequency * 2.0 * time).sin() * 0.15;
    tone * envelope
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timer::Settings;

    #[test]
    fn noise_is_only_requested_during_running_focus() {
        let mut snapshot = Snapshot {
            show_settings: false,
            settings: Settings::default(),
            mode: Mode::Focus,
            remaining: Duration::from_secs(60),
            phase_total: Duration::from_secs(60),
            running: true,
            completed_in_cycle: 0,
            completed_today: 0,
            completed_phases: 0,
            notice: None,
            sound_preview_seq: 0,
            sound_preview: None,
        };
        assert_eq!(wanted_noise(&snapshot), FocusSound::Brown);
        snapshot.running = false;
        assert_eq!(wanted_noise(&snapshot), FocusSound::Off);
        snapshot.running = true;
        snapshot.mode = Mode::ShortBreak;
        assert_eq!(wanted_noise(&snapshot), FocusSound::Off);
        snapshot.mode = Mode::LongBreak;
        assert_eq!(wanted_noise(&snapshot), FocusSound::Off);
    }

    #[test]
    fn completion_cues_are_finite_and_below_clipping() {
        let sample_rate = NonZero::new(44_100).unwrap();
        for sound in [
            FinishSound::Gong,
            FinishSound::Chime,
            FinishSound::Bell,
            FinishSound::Wood,
            FinishSound::Pulse,
        ] {
            let samples = completion_samples(sound, sample_rate);
            let values = samples.collect::<Vec<_>>();
            assert!(!values.is_empty());
            assert!(values
                .iter()
                .all(|sample| sample.is_finite() && sample.abs() < 1.0));
            assert!(values.iter().any(|sample| sample.abs() > 0.01));
        }
    }

    #[test]
    fn requested_fade_reaches_silence_then_ends_source() {
        let sample_rate = NonZero::new(10).unwrap();
        let samples = SamplesBuffer::new(NonZero::new(1).unwrap(), sample_rate, vec![1.0; 10]);
        let (mut source, request) = FadeOnRequest::new(samples, Duration::from_millis(300));

        assert_eq!(source.next(), Some(1.0));
        request.store(true, Ordering::Relaxed);
        assert_eq!(source.next(), Some(2.0 / 3.0));
        assert_eq!(source.next(), Some(1.0 / 3.0));
        assert_eq!(source.next(), Some(0.0));
        assert_eq!(source.next(), None);
    }

    #[test]
    fn noise_preview_lasts_three_seconds_with_only_a_short_end_fade() {
        let sample_rate = NonZero::new(1_000).unwrap();
        let samples = SamplesBuffer::new(NonZero::new(1).unwrap(), sample_rate, vec![1.0; 4_000]);
        let preview = PreviewClip::new(samples, NOISE_PREVIEW_DURATION).collect::<Vec<_>>();
        assert_eq!(preview.len(), 3_000);
        assert_eq!(preview[0], 1.0);
        assert_eq!(preview[2_000], 1.0);
        assert_eq!(preview[2_999], 0.0);
    }

    #[test]
    fn finish_preview_stays_short() {
        let sample_rate = NonZero::new(1_000).unwrap();
        let samples = SamplesBuffer::new(NonZero::new(1).unwrap(), sample_rate, vec![1.0; 2_000]);
        let preview = PreviewClip::new(samples, FINISH_PREVIEW_DURATION).collect::<Vec<_>>();
        assert_eq!(preview.len(), 1_000);
        assert_eq!(preview[999], 0.0);
    }

    #[test]
    fn preview_duration_accounts_for_every_channel() {
        let sample_rate = NonZero::new(1_000).unwrap();
        let channels = NonZero::new(2).unwrap();
        let samples = SamplesBuffer::new(channels, sample_rate, vec![1.0; 8_000]);
        let preview = PreviewClip::new(samples, NOISE_PREVIEW_DURATION).collect::<Vec<_>>();

        assert_eq!(preview.len(), 6_000);
        assert_eq!(preview[5_000], 1.0);
        assert_eq!(preview[5_999], 0.0);
    }
}
