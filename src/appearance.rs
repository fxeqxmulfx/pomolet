//! Linux color scheme updates from the XDG Settings portal.

use iced::{theme, Subscription};
use std::time::Duration;
use tokio::sync::mpsc;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

pub fn subscription() -> Subscription<Option<theme::Mode>> {
    Subscription::run(portal_changes)
}

fn portal_changes() -> impl iced::futures::Stream<Item = Option<theme::Mode>> {
    let (sender, receiver) = mpsc::unbounded_channel();
    std::thread::spawn(move || watch_portal(sender));

    iced::futures::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|mode| (mode, receiver))
    })
}

fn watch_portal(sender: mpsc::UnboundedSender<Option<theme::Mode>>) {
    let mut last_error = String::new();
    loop {
        match watch_connection(&sender) {
            Ok(()) => last_error.clear(),
            Err(error) => {
                let error = error.to_string();
                if error != last_error {
                    eprintln!("Color scheme portal: {error}");
                    last_error = error;
                }
            }
        }
        if sender.is_closed() {
            return;
        }
        std::thread::sleep(Duration::from_secs(10));
    }
}

fn watch_connection(sender: &mpsc::UnboundedSender<Option<theme::Mode>>) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let proxy = Proxy::new(&connection, DESTINATION, PATH, INTERFACE)?;
    let mut signals =
        proxy.receive_signal_with_args("SettingChanged", &[(0, NAMESPACE), (1, KEY)])?;

    // Subscribe first, then read. A change during startup will still be delivered.
    if let Ok(value) = read_scheme(&proxy) {
        if sender.send(scheme_mode(value)).is_err() {
            return Ok(());
        }
    }

    for signal in &mut signals {
        let Ok((_, _, value)) = signal.body().deserialize::<(String, String, OwnedValue)>() else {
            continue;
        };
        if sender.send(scheme_mode(value)).is_err() {
            return Ok(());
        }
    }

    Ok(())
}

fn read_scheme(proxy: &Proxy<'_>) -> zbus::Result<OwnedValue> {
    proxy
        .call("ReadOne", &(NAMESPACE, KEY))
        .or_else(|_| proxy.call("Read", &(NAMESPACE, KEY)))
}

fn scheme_mode(value: OwnedValue) -> Option<theme::Mode> {
    let mut value: Value<'_> = value.into();
    while let Value::Value(inner) = value {
        value = *inner;
    }
    match value {
        Value::U32(1) => Some(theme::Mode::Dark),
        Value::U32(2) => Some(theme::Mode::Light),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_scheme_handles_nested_variants_and_no_preference() {
        let dark = Value::Value(Box::new(Value::Value(Box::new(Value::U32(1)))));
        assert_eq!(
            scheme_mode(dark.try_into().unwrap()),
            Some(theme::Mode::Dark)
        );
        assert_eq!(
            scheme_mode(Value::U32(2).try_into().unwrap()),
            Some(theme::Mode::Light)
        );
        assert_eq!(scheme_mode(Value::U32(0).try_into().unwrap()), None);
    }
}
