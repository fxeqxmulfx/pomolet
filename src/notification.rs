//! Desktop notifications for naturally completed timer phases on Linux.

use crate::core::Snapshot;
use crate::timer::Mode;
use std::collections::HashMap;
use std::thread;
use tokio::sync::mpsc::UnboundedReceiver;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

const DESTINATION: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

pub fn spawn(updates: UnboundedReceiver<Snapshot>) {
    thread::Builder::new()
        .name("pomolet-notifications".into())
        .spawn(move || run(updates))
        .expect("failed to start notification subscriber");
}

fn run(mut updates: UnboundedReceiver<Snapshot>) {
    let mut tracker = NotificationTracker::default();
    while let Some(snapshot) = updates.blocking_recv() {
        if let Some(notification) = tracker.observe(&snapshot) {
            if let Err(error) = send(notification) {
                eprintln!("Desktop notification unavailable: {error}");
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Notification {
    summary: &'static str,
    body: &'static str,
}

#[derive(Default)]
struct NotificationTracker {
    last_completed: Option<u64>,
}

impl NotificationTracker {
    fn observe(&mut self, snapshot: &Snapshot) -> Option<Notification> {
        let changed = self
            .last_completed
            .replace(snapshot.completed_phases)
            .is_some_and(|previous| previous != snapshot.completed_phases);
        if !changed {
            return None;
        }

        Some(match snapshot.mode {
            Mode::Focus => Notification {
                summary: "Break complete",
                body: "Ready to focus again.",
            },
            Mode::ShortBreak => Notification {
                summary: "Focus complete",
                body: "Time for a short break.",
            },
            Mode::LongBreak => Notification {
                summary: "Focus complete",
                body: "Time for a long break.",
            },
        })
    }
}

fn send(notification: Notification) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let proxy = Proxy::new(&connection, DESTINATION, PATH, DESTINATION)?;
    let _: u32 = proxy.call(
        "Notify",
        &(
            "Pomolet",
            0u32,
            "",
            notification.summary,
            notification.body,
            Vec::<&str>::new(),
            HashMap::<&str, OwnedValue>::new(),
            -1i32,
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timer::Settings;
    use std::time::Duration;

    #[test]
    fn only_natural_completions_produce_one_notification() {
        let mut snapshot = Snapshot {
            show_settings: false,
            settings: Settings::default(),
            mode: Mode::Focus,
            remaining: Duration::from_secs(60),
            phase_total: Duration::from_secs(60),
            running: false,
            completed_in_cycle: 0,
            completed_today: 0,
            completed_phases: 0,
            notice: None,
            sound_preview_seq: 0,
            sound_preview: None,
        };
        let mut tracker = NotificationTracker::default();
        assert_eq!(tracker.observe(&snapshot), None);

        snapshot.mode = Mode::ShortBreak;
        assert_eq!(tracker.observe(&snapshot), None); // Skip keeps the phase count.

        snapshot.completed_phases = 1;
        assert_eq!(
            tracker.observe(&snapshot),
            Some(Notification {
                summary: "Focus complete",
                body: "Time for a short break.",
            })
        );
        assert_eq!(tracker.observe(&snapshot), None);

        snapshot.mode = Mode::Focus;
        snapshot.completed_phases = 2;
        assert_eq!(
            tracker.observe(&snapshot),
            Some(Notification {
                summary: "Break complete",
                body: "Ready to focus again.",
            })
        );
    }

    #[test]
    fn startup_history_is_silent_and_fourth_focus_announces_long_break() {
        let mut snapshot = Snapshot {
            show_settings: false,
            settings: Settings::default(),
            mode: Mode::Focus,
            remaining: Duration::from_secs(60),
            phase_total: Duration::from_secs(60),
            running: false,
            completed_in_cycle: 3,
            completed_today: 3,
            completed_phases: 7,
            notice: None,
            sound_preview_seq: 0,
            sound_preview: None,
        };
        let mut tracker = NotificationTracker::default();
        assert_eq!(tracker.observe(&snapshot), None);

        snapshot.mode = Mode::LongBreak;
        snapshot.completed_phases = 8;
        assert_eq!(
            tracker.observe(&snapshot),
            Some(Notification {
                summary: "Focus complete",
                body: "Time for a long break.",
            })
        );
        snapshot.settings.focus_sound = crate::timer::FocusSound::Violet;
        assert_eq!(tracker.observe(&snapshot), None);
    }

    #[test]
    #[ignore = "requires a desktop notification service"]
    fn sends_to_desktop_notification_service() {
        send(Notification {
            summary: "Pomolet test",
            body: "Desktop notifications are working.",
        })
        .unwrap();
    }
}
