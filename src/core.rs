use crate::timer::{FinishSound, FocusSound, Mode, Settings, Timer};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{self as async_mpsc, UnboundedReceiver, UnboundedSender};

/// Commands are the only input to the timer core.
#[derive(Debug, Clone, Copy)]
pub enum Command {
    StartPause,
    Reset,
    Skip,
    SelectMode(Mode),
    Adjust(Mode, i32),
    SetFocusSound(FocusSound),
    SetFinishSound(FinishSound),
    ToggleSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundPreview {
    Focus(FocusSound),
    Finish(FinishSound),
}

/// Immutable state delivered to every subscriber.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub show_settings: bool,
    pub settings: Settings,
    pub mode: Mode,
    pub remaining: Duration,
    pub phase_total: Duration,
    pub running: bool,
    pub completed_in_cycle: u8,
    pub completed_today: u32,
    pub completed_phases: u64,
    pub notice: Option<&'static str>,
    pub sound_preview_seq: u64,
    pub sound_preview: Option<SoundPreview>,
}

impl Snapshot {
    pub fn display_time(&self) -> String {
        let seconds = self.remaining.as_secs_f64().ceil() as u64;
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }

    pub fn progress(&self) -> f32 {
        if self.phase_total.is_zero() {
            return 0.0;
        }
        (1.0 - self.remaining.as_secs_f32() / self.phase_total.as_secs_f32()).clamp(0.0, 1.0)
    }
}

enum Input {
    Notify(Command),
    Subscribe(UnboundedSender<Snapshot>),
}

#[derive(Clone)]
pub struct CoreHandle {
    sender: Sender<Input>,
    id: u64,
}

static NEXT_CORE_ID: AtomicU64 = AtomicU64::new(1);

impl PartialEq for CoreHandle {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for CoreHandle {}

impl Hash for CoreHandle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl CoreHandle {
    pub fn spawn() -> (Self, Snapshot) {
        let state = CoreState::load();
        let initial = state.snapshot();
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("pomolet-core".into())
            .spawn(move || state.run(receiver))
            .expect("failed to start timer core");
        (
            Self {
                sender,
                id: NEXT_CORE_ID.fetch_add(1, Ordering::Relaxed),
            },
            initial,
        )
    }

    pub fn notify(&self, command: Command) {
        let _ = self.sender.send(Input::Notify(command));
    }

    pub fn subscribe(&self) -> UnboundedReceiver<Snapshot> {
        let (sender, receiver) = async_mpsc::unbounded_channel();
        let _ = self.sender.send(Input::Subscribe(sender));
        receiver
    }
}

struct CoreState {
    timer: Timer,
    day: String,
    show_settings: bool,
    subscribers: Vec<UnboundedSender<Snapshot>>,
    sound_preview_seq: u64,
    sound_preview: Option<SoundPreview>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct SavedState {
    settings: Settings,
    day: String,
    completed_today: u32,
    completed_total: u32,
}

impl CoreState {
    fn load() -> Self {
        let today = today();
        let mut saved = storage_path()
            .and_then(read_saved)
            .or_else(|| previous_storage_path().and_then(read_saved))
            .or_else(|| legacy_storage_path().and_then(read_saved))
            .unwrap_or_default();
        saved.settings.focus = saved.settings.focus.clamp(1, 120);
        saved.settings.short_break = saved.settings.short_break.clamp(1, 60);
        saved.settings.long_break = saved.settings.long_break.clamp(1, 60);
        if saved.day != today {
            saved.completed_today = 0;
        }
        Self {
            timer: Timer::new(saved.settings, saved.completed_today, saved.completed_total),
            day: today,
            show_settings: false,
            subscribers: Vec::new(),
            sound_preview_seq: 0,
            sound_preview: None,
        }
    }

    fn run(mut self, receiver: mpsc::Receiver<Input>) {
        loop {
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(Input::Notify(command)) => {
                    self.handle(command);
                    self.broadcast();
                }
                Ok(Input::Subscribe(sender)) => {
                    if sender.send(self.snapshot()).is_ok() {
                        self.subscribers.push(sender);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }

            if self.refresh_day() {
                self.save();
                self.broadcast();
            }
            if self.timer.is_running() {
                let shown_before = self.timer.remaining.as_secs_f64().ceil() as u64;
                let finished = self.timer.tick(Instant::now());
                if finished {
                    self.save();
                }
                let shown_after = self.timer.remaining.as_secs_f64().ceil() as u64;
                if finished || shown_after != shown_before {
                    self.broadcast();
                }
            }
        }
    }

    fn handle(&mut self, command: Command) {
        let completed_before = self.timer.completed_total;
        match command {
            Command::StartPause => self.timer.start_or_pause(Instant::now()),
            Command::Reset => self.timer.reset(),
            Command::Skip => self.timer.skip(),
            Command::SelectMode(mode) => self.timer.select_mode(mode),
            Command::Adjust(mode, delta) => {
                self.timer.adjust(mode, delta);
                self.save();
            }
            Command::SetFocusSound(sound) => {
                self.timer.settings.focus_sound = sound;
                self.sound_preview_seq = self.sound_preview_seq.wrapping_add(1);
                self.sound_preview = Some(SoundPreview::Focus(sound));
                self.save();
            }
            Command::SetFinishSound(sound) => {
                self.timer.settings.finish_sound = sound;
                self.sound_preview_seq = self.sound_preview_seq.wrapping_add(1);
                self.sound_preview = Some(SoundPreview::Finish(sound));
                self.save();
            }
            Command::ToggleSettings => self.show_settings = !self.show_settings,
        }
        if self.timer.completed_total != completed_before {
            self.save();
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            show_settings: self.show_settings,
            settings: self.timer.settings,
            mode: self.timer.mode,
            remaining: self.timer.remaining,
            phase_total: self.timer.phase_total,
            running: self.timer.is_running(),
            completed_in_cycle: self.timer.completed_in_cycle,
            completed_today: self.timer.completed_today,
            completed_phases: self.timer.completed_phases,
            notice: self.timer.notice,
            sound_preview_seq: self.sound_preview_seq,
            sound_preview: self.sound_preview,
        }
    }

    fn broadcast(&mut self) {
        let snapshot = self.snapshot();
        self.subscribers
            .retain(|subscriber| subscriber.send(snapshot.clone()).is_ok());
    }

    fn refresh_day(&mut self) -> bool {
        let current = today();
        if self.day == current {
            return false;
        }
        self.day = current;
        self.timer.completed_today = 0;
        true
    }

    fn save(&self) {
        let Some(path) = storage_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            if fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let state = SavedState {
            settings: self.timer.settings,
            day: self.day.clone(),
            completed_today: self.timer.completed_today,
            completed_total: self.timer.completed_total,
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&state) {
            let _ = fs::write(path, bytes);
        }
    }
}

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn storage_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("pomolet").join("state.json"))
}

fn previous_storage_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("pomodoro").join("state.json"))
}

fn legacy_storage_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("animecat-pomodoro").join("state.json"))
}

fn read_saved(path: PathBuf) -> Option<SavedState> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn core_publishes_command_results_to_every_subscriber() {
        let mut state = CoreState {
            timer: Timer::new(Settings::default(), 0, 0),
            day: today(),
            show_settings: false,
            subscribers: Vec::new(),
            sound_preview_seq: 0,
            sound_preview: None,
        };
        let (first_tx, mut first_rx) = async_mpsc::unbounded_channel();
        let (second_tx, mut second_rx) = async_mpsc::unbounded_channel();
        state.subscribers.extend([first_tx, second_tx]);

        state.handle(Command::SelectMode(Mode::LongBreak));
        state.broadcast();

        assert_eq!(first_rx.try_recv().unwrap().mode, Mode::LongBreak);
        assert_eq!(second_rx.try_recv().unwrap().mode, Mode::LongBreak);

        state.handle(Command::ToggleSettings);
        state.broadcast();
        assert!(first_rx.try_recv().unwrap().show_settings);
        assert!(second_rx.try_recv().unwrap().show_settings);
    }

    #[test]
    fn core_handle_delivers_notifications_through_subscription() {
        let (core, _) = CoreHandle::spawn();
        let mut subscriber = core.subscribe();
        let deadline = Instant::now() + Duration::from_secs(2);
        while subscriber.try_recv().is_err() {
            assert!(
                Instant::now() < deadline,
                "initial snapshot was not delivered"
            );
            thread::sleep(Duration::from_millis(5));
        }

        core.notify(Command::SelectMode(Mode::LongBreak));
        loop {
            if let Ok(snapshot) = subscriber.try_recv() {
                assert_eq!(snapshot.mode, Mode::LongBreak);
                break;
            }
            assert!(Instant::now() < deadline, "core command was not published");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn snapshot_rounds_remaining_time_up_and_bounds_progress() {
        let state = CoreState {
            timer: Timer::new(Settings::default(), 0, 0),
            day: today(),
            show_settings: false,
            subscribers: Vec::new(),
            sound_preview_seq: 0,
            sound_preview: None,
        };
        let mut snapshot = state.snapshot();
        snapshot.phase_total = Duration::from_secs(10);
        snapshot.remaining = Duration::from_millis(1_001);
        assert_eq!(snapshot.display_time(), "00:02");
        assert!((snapshot.progress() - 0.8999).abs() < 0.001);

        snapshot.remaining = Duration::from_secs(12);
        assert_eq!(snapshot.progress(), 0.0);
        snapshot.remaining = Duration::ZERO;
        assert_eq!(snapshot.display_time(), "00:00");
        assert_eq!(snapshot.progress(), 1.0);
        snapshot.phase_total = Duration::ZERO;
        assert_eq!(snapshot.progress(), 0.0);
    }

    #[test]
    fn day_rollover_resets_only_daily_count() {
        let mut state = CoreState {
            timer: Timer::new(Settings::default(), 4, 17),
            day: "2000-01-01".into(),
            show_settings: false,
            subscribers: Vec::new(),
            sound_preview_seq: 0,
            sound_preview: None,
        };
        let now = Instant::now();
        state.timer.start_or_pause(now);
        let deadline = state.timer.deadline;

        assert!(state.refresh_day());
        assert_eq!(state.timer.completed_today, 0);
        assert_eq!(state.timer.completed_total, 17);
        assert_eq!(state.timer.deadline, deadline);
        assert!(!state.refresh_day());
    }
}
