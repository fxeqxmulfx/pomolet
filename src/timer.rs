use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Focus => "Focus",
            Self::ShortBreak => "Short break",
            Self::LongBreak => "Long break",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FocusSound {
    Off,
    #[default]
    Brown,
    Pink,
    White,
    Blue,
    Violet,
}

impl FocusSound {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Brown => "Brown",
            Self::Pink => "Pink",
            Self::White => "White",
            Self::Blue => "Blue",
            Self::Violet => "Violet",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FinishSound {
    Off,
    #[default]
    Gong,
    Chime,
    Bell,
    Wood,
    Pulse,
}

impl FinishSound {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Gong => "Gong",
            Self::Chime => "Chime",
            Self::Bell => "Bell",
            Self::Wood => "Wood",
            Self::Pulse => "Pulse",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub focus: u32,
    pub short_break: u32,
    pub long_break: u32,
    pub focus_sound: FocusSound,
    pub finish_sound: FinishSound,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            focus: 25,
            short_break: 5,
            long_break: 15,
            focus_sound: FocusSound::default(),
            finish_sound: FinishSound::default(),
        }
    }
}

impl Settings {
    pub fn minutes(self, mode: Mode) -> u32 {
        match mode {
            Mode::Focus => self.focus,
            Mode::ShortBreak => self.short_break,
            Mode::LongBreak => self.long_break,
        }
    }

    pub fn adjust(&mut self, mode: Mode, delta: i32) {
        let value = match mode {
            Mode::Focus => &mut self.focus,
            Mode::ShortBreak => &mut self.short_break,
            Mode::LongBreak => &mut self.long_break,
        };
        let max = if mode == Mode::Focus { 120 } else { 60 };
        *value = (*value as i32 + delta).clamp(1, max) as u32;
    }
}

pub struct Timer {
    pub settings: Settings,
    pub mode: Mode,
    pub remaining: Duration,
    pub phase_total: Duration,
    pub deadline: Option<Instant>,
    pub completed_in_cycle: u8,
    pub completed_today: u32,
    pub completed_total: u32,
    pub completed_phases: u64,
    pub notice: Option<&'static str>,
}

impl Timer {
    pub fn new(settings: Settings, completed_today: u32, completed_total: u32) -> Self {
        let phase_total = Duration::from_secs(settings.focus as u64 * 60);
        Self {
            settings,
            mode: Mode::Focus,
            remaining: phase_total,
            phase_total,
            deadline: None,
            completed_in_cycle: 0,
            completed_today,
            completed_total,
            completed_phases: 0,
            notice: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.deadline.is_some()
    }

    pub fn start_or_pause(&mut self, now: Instant) {
        if let Some(deadline) = self.deadline.take() {
            self.remaining = deadline.saturating_duration_since(now);
            if self.remaining.is_zero() {
                self.finish();
            }
        } else {
            if self.remaining.is_zero() {
                self.reset();
            }
            self.deadline = Some(now + self.remaining);
            self.notice = None;
        }
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let Some(deadline) = self.deadline else {
            return false;
        };
        self.remaining = deadline.saturating_duration_since(now);
        if self.remaining.is_zero() {
            self.deadline = None;
            self.finish();
            return true;
        }
        false
    }

    pub fn reset(&mut self) {
        self.deadline = None;
        self.phase_total = self.duration_for(self.mode);
        self.remaining = self.phase_total;
        self.notice = None;
    }

    pub fn select_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.reset();
    }

    pub fn skip(&mut self) {
        let next = match self.mode {
            Mode::Focus if self.completed_in_cycle == 3 => Mode::LongBreak,
            Mode::Focus => Mode::ShortBreak,
            Mode::ShortBreak | Mode::LongBreak => Mode::Focus,
        };
        self.select_mode(next);
    }

    pub fn adjust(&mut self, mode: Mode, delta: i32) {
        self.settings.adjust(mode, delta);
        if self.mode == mode && !self.is_running() {
            self.reset();
        }
    }

    fn duration_for(&self, mode: Mode) -> Duration {
        Duration::from_secs(self.settings.minutes(mode) as u64 * 60)
    }

    fn finish(&mut self) {
        self.completed_phases = self.completed_phases.wrapping_add(1);
        let next = match self.mode {
            Mode::Focus => {
                self.completed_today += 1;
                self.completed_total += 1;
                self.completed_in_cycle = (self.completed_in_cycle + 1) % 4;
                self.notice = Some("Focus complete. Time for a break.");
                if self.completed_in_cycle == 0 {
                    Mode::LongBreak
                } else {
                    Mode::ShortBreak
                }
            }
            Mode::ShortBreak | Mode::LongBreak => {
                self.notice = Some("Break complete. Ready to focus.");
                Mode::Focus
            }
        };
        self.mode = next;
        self.phase_total = self.duration_for(next);
        self.remaining = self.phase_total;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_time_uses_deadline_and_pause_preserves_remaining() {
        let now = Instant::now();
        let mut timer = Timer::new(Settings::default(), 0, 0);
        timer.start_or_pause(now);
        timer.tick(now + Duration::from_secs(75));
        assert_eq!(timer.remaining.as_secs(), 23 * 60 + 45);
        timer.start_or_pause(now + Duration::from_secs(75));
        assert!(!timer.is_running());
        timer.start_or_pause(now + Duration::from_secs(100));
        timer.tick(now + Duration::from_secs(101));
        assert_eq!(timer.remaining.as_secs(), 23 * 60 + 44);
    }

    #[test]
    fn every_fourth_focus_gets_long_break_and_skip_does_not_count() {
        let now = Instant::now();
        let mut timer = Timer::new(
            Settings {
                focus: 1,
                short_break: 1,
                long_break: 1,
                ..Settings::default()
            },
            0,
            0,
        );
        timer.skip();
        assert_eq!(timer.completed_today, 0);
        assert_eq!(timer.completed_phases, 0);
        timer.skip();
        for session in 1..=4 {
            timer.start_or_pause(now);
            assert!(timer.tick(now + Duration::from_secs(60)));
            assert_eq!(timer.completed_today, session);
            assert_eq!(timer.completed_phases, session as u64);
            assert_eq!(
                timer.mode,
                if session == 4 {
                    Mode::LongBreak
                } else {
                    Mode::ShortBreak
                }
            );
            timer.skip();
        }
        assert_eq!(timer.mode, Mode::Focus);
        assert_eq!(timer.completed_in_cycle, 0);
    }

    #[test]
    fn completing_a_break_emits_a_phase_completion() {
        let now = Instant::now();
        let mut timer = Timer::new(Settings::default(), 0, 0);
        timer.select_mode(Mode::ShortBreak);
        timer.start_or_pause(now);
        assert!(timer.tick(now + Duration::from_secs(5 * 60)));
        assert_eq!(timer.completed_phases, 1);
        assert_eq!(timer.completed_today, 0);
        assert_eq!(timer.mode, Mode::Focus);
    }

    #[test]
    fn old_settings_gain_default_audio_preferences() {
        let settings: Settings =
            serde_json::from_str(r#"{"focus":25,"short_break":5,"long_break":15}"#).unwrap();
        assert_eq!(settings.focus_sound, FocusSound::Brown);
        assert_eq!(settings.finish_sound, FinishSound::Gong);
    }

    #[test]
    fn duration_adjustments_respect_each_modes_limits() {
        let mut settings = Settings::default();
        settings.adjust(Mode::Focus, -100);
        settings.adjust(Mode::ShortBreak, 100);
        settings.adjust(Mode::LongBreak, -100);
        assert_eq!(settings.focus, 1);
        assert_eq!(settings.short_break, 60);
        assert_eq!(settings.long_break, 1);

        settings.adjust(Mode::Focus, 200);
        assert_eq!(settings.focus, 120);
        assert_eq!(settings.short_break, 60);
    }

    #[test]
    fn changing_duration_does_not_move_an_active_deadline() {
        let now = Instant::now();
        let mut timer = Timer::new(Settings::default(), 0, 0);
        timer.start_or_pause(now);
        assert!(!timer.tick(now + Duration::from_secs(60)));

        timer.adjust(Mode::Focus, 5);
        assert_eq!(timer.settings.focus, 30);
        assert_eq!(timer.phase_total, Duration::from_secs(25 * 60));
        assert_eq!(timer.remaining, Duration::from_secs(24 * 60));
        assert_eq!(timer.deadline, Some(now + Duration::from_secs(25 * 60)));

        timer.reset();
        assert!(!timer.is_running());
        assert_eq!(timer.remaining, Duration::from_secs(30 * 60));
    }

    #[test]
    fn pausing_at_the_deadline_finishes_once() {
        let now = Instant::now();
        let mut timer = Timer::new(
            Settings {
                focus: 1,
                ..Settings::default()
            },
            2,
            10,
        );
        timer.start_or_pause(now);
        timer.start_or_pause(now + Duration::from_secs(60));

        assert!(!timer.is_running());
        assert_eq!(timer.mode, Mode::ShortBreak);
        assert_eq!(timer.completed_today, 3);
        assert_eq!(timer.completed_total, 11);
        assert_eq!(timer.completed_phases, 1);
        assert!(!timer.tick(now + Duration::from_secs(61)));
        assert_eq!(timer.completed_phases, 1);
    }

    #[test]
    fn skip_and_reset_stop_running_without_counting_a_completion() {
        let now = Instant::now();
        let mut timer = Timer::new(Settings::default(), 0, 0);
        timer.start_or_pause(now);
        timer.skip();
        assert_eq!(timer.mode, Mode::ShortBreak);
        assert_eq!(timer.remaining, Duration::from_secs(5 * 60));
        assert!(!timer.is_running());

        timer.start_or_pause(now);
        timer.reset();
        assert!(!timer.is_running());
        assert_eq!(timer.remaining, Duration::from_secs(5 * 60));
        assert_eq!(timer.completed_today, 0);
        assert_eq!(timer.completed_phases, 0);
    }
}
