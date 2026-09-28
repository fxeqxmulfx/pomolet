#[cfg(target_os = "linux")]
mod appearance;
mod audio;
mod core;
#[cfg(target_os = "linux")]
mod notification;
mod timer;

use core::{Command, CoreHandle, Snapshot};
use iced::widget::{
    button, column, container, progress_bar, responsive, row, scrollable, text, Space,
};
use iced::{
    theme, Alignment, Background, Border, Color, Element, Font, Length, Shadow, Size, Subscription,
    Task, Theme,
};
use timer::{FinishSound, FocusSound, Mode};

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("Pomolet")
        .subscription(App::subscription)
        .theme(App::theme)
        .font(include_bytes!("../assets/fonts/Inter-Regular.otf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-SemiBold.otf").as_slice())
        .font(include_bytes!("../assets/fonts/InterDisplay-Bold.otf").as_slice())
        .default_font(Font::with_name("Inter"))
        .window_size((540.0, 760.0))
        .antialiasing(true)
        .run()
}

#[derive(Debug, Clone)]
enum Message {
    Notify(Command),
    Core(Snapshot),
    SystemTheme(theme::Mode),
    #[cfg(target_os = "linux")]
    PortalTheme(Option<theme::Mode>),
}

struct App {
    core: CoreHandle,
    snapshot: Snapshot,
    theme_mode: theme::Mode,
    #[cfg(target_os = "linux")]
    portal_theme: Option<theme::Mode>,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let (core, snapshot) = CoreHandle::spawn();
        audio::spawn(core.subscribe());
        #[cfg(target_os = "linux")]
        notification::spawn(core.subscribe());
        (
            Self {
                core,
                snapshot,
                theme_mode: theme::Mode::Light,
                #[cfg(target_os = "linux")]
                portal_theme: None,
            },
            iced::system::theme().map(Message::SystemTheme),
        )
    }

    fn theme(&self) -> Theme {
        if self.effective_theme_mode() == theme::Mode::Dark {
            Theme::Dark
        } else {
            Theme::Light
        }
    }

    fn effective_theme_mode(&self) -> theme::Mode {
        #[cfg(target_os = "linux")]
        if let Some(mode) = self.portal_theme {
            return mode;
        }

        self.theme_mode
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Notify(command) => self.core.notify(command),
            Message::Core(snapshot) => self.snapshot = snapshot,
            Message::SystemTheme(mode) => self.theme_mode = mode,
            #[cfg(target_os = "linux")]
            Message::PortalTheme(mode) => self.portal_theme = mode,
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let core = Subscription::run_with(self.core.clone(), |core| {
            iced::futures::stream::unfold(core.subscribe(), |mut receiver| async move {
                receiver
                    .recv()
                    .await
                    .map(|snapshot| (Message::Core(snapshot), receiver))
            })
        });
        let native_theme = iced::system::theme_changes().map(Message::SystemTheme);

        #[cfg(target_os = "linux")]
        return Subscription::batch([
            core,
            native_theme,
            appearance::subscription().map(Message::PortalTheme),
        ]);

        #[cfg(not(target_os = "linux"))]
        Subscription::batch([core, native_theme])
    }

    fn view(&self) -> Element<'_, Message> {
        responsive(|size| self.view_at_size(size)).into()
    }

    fn view_at_size(&self, size: Size) -> Element<'_, Message> {
        let palette = Palette::for_mode(self.effective_theme_mode());
        let expanded = size.width >= 850.0 && size.height >= 780.0;
        let header = row![
            column![
                text("Pomolet")
                    .size(if expanded { 36 } else { 32 })
                    .font(display_bold())
                    .color(palette.text_primary),
                text("Actionless vision is a daydream, visionless action is a nightmare.")
                    .size(14)
                    .width(Length::Fill)
                    .color(palette.text_secondary),
            ]
            .spacing(3)
            .width(Length::Fill),
            button(
                text(if self.snapshot.show_settings {
                    "Back"
                } else {
                    "Settings"
                })
                .size(13)
            )
            .on_press(Message::Notify(Command::ToggleSettings))
            .padding([11, 17])
            .style(move |_, status| soft_button(status, palette)),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        let content: Element<'_, Message> = if self.snapshot.show_settings {
            self.settings_view(palette)
        } else {
            self.timer_view(expanded, palette)
        };

        let body = column![header, content].spacing(28).width(Length::Fill);
        let page = container(body)
            .padding(if expanded { [36, 34] } else { [24, 28] })
            .width(Length::Fill)
            .max_width(if expanded { 680 } else { 500 });
        let centered_page = container(page).width(Length::Fill).center_x(Length::Fill);

        if !self.snapshot.show_settings && size.height >= 760.0 {
            container(centered_page)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_y(Length::Fill)
                .style(move |_| backdrop(palette))
                .into()
        } else {
            container(
                scrollable(centered_page)
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| backdrop(palette))
            .into()
        }
    }

    fn timer_view(&self, expanded: bool, palette: Palette) -> Element<'_, Message> {
        let mode_buttons = container(
            row![
                mode_button(Mode::Focus, self.snapshot.mode, palette),
                mode_button(Mode::ShortBreak, self.snapshot.mode, palette),
                mode_button(Mode::LongBreak, self.snapshot.mode, palette),
            ]
            .spacing(4)
            .width(Length::Fill),
        )
        .padding(5)
        .width(Length::Fill)
        .style(move |_| panel(palette.panel_alt, 21.0));

        let status = self.snapshot.notice.unwrap_or(match self.snapshot.mode {
            Mode::Focus => "One task at a time.",
            Mode::ShortBreak => "Take a short pause.",
            Mode::LongBreak => "Take time to recharge.",
        });
        let percent = (self.snapshot.progress() * 100.0).round() as u32;
        let progress = column![
            row![
                text(if self.snapshot.running {
                    "IN PROGRESS"
                } else {
                    "SESSION PROGRESS"
                })
                .size(11)
                .font(semibold())
                .color(palette.text_secondary),
                Space::new().width(Length::Fill),
                text(format!("{percent}%"))
                    .size(12)
                    .color(palette.text_secondary),
            ],
            progress_bar(0.0..=1.0, self.snapshot.progress())
                .girth(7)
                .style(move |_| iced::widget::progress_bar::Style {
                    background: palette.track.into(),
                    bar: palette.accent.into(),
                    border: Border {
                        radius: 8.0.into(),
                        ..Border::default()
                    },
                }),
        ]
        .spacing(9);

        let timer_card = container(
            column![
                column![
                    text(self.snapshot.mode.label().to_uppercase())
                        .size(12)
                        .font(semibold())
                        .color(palette.accent),
                    text(self.snapshot.display_time())
                        .size(if expanded { 116 } else { 94 })
                        .font(display_bold())
                        .color(palette.text_primary),
                    text(status).size(15).color(palette.text_secondary),
                ]
                .spacing(7)
                .align_x(Alignment::Center)
                .width(Length::Fill),
                progress,
            ]
            .spacing(if expanded { 36 } else { 30 }),
        )
        .width(Length::Fill)
        .padding(if expanded { [34, 34] } else { [30, 26] })
        .style(move |_| card(30.0, palette));

        let primary_label = if self.snapshot.running {
            "Pause"
        } else if self.snapshot.remaining < self.snapshot.phase_total {
            "Resume"
        } else {
            "Start"
        };
        let controls = column![
            button(
                container(text(primary_label).size(17).font(semibold()))
                    .width(Length::Fill)
                    .center_x(Length::Fill)
            )
            .on_press(Message::Notify(Command::StartPause))
            .padding(17)
            .width(Length::Fill)
            .style(move |_, status| primary_button(status, palette)),
            row![
                button(
                    container(text("Reset").size(14))
                        .width(Length::Fill)
                        .center_x(Length::Fill)
                )
                .on_press(Message::Notify(Command::Reset))
                .width(Length::Fill)
                .padding(14)
                .style(move |_, status| soft_button(status, palette)),
                button(
                    container(text("Skip").size(14))
                        .width(Length::Fill)
                        .center_x(Length::Fill)
                )
                .on_press(Message::Notify(Command::Skip))
                .width(Length::Fill)
                .padding(14)
                .style(move |_, status| soft_button(status, palette)),
            ]
            .spacing(12)
            .width(Length::Fill),
        ]
        .spacing(12);

        let stats = container(
            row![
                column![
                    text("TODAY")
                        .size(11)
                        .font(semibold())
                        .color(palette.accent),
                    text(format!(
                        "{} focus {}",
                        self.snapshot.completed_today,
                        if self.snapshot.completed_today == 1 {
                            "session"
                        } else {
                            "sessions"
                        }
                    ))
                    .size(17)
                    .font(semibold())
                    .color(palette.text_primary),
                ]
                .spacing(4),
                Space::new().width(Length::Fill),
                cycle_indicator(self.snapshot.completed_in_cycle, palette),
            ]
            .align_y(Alignment::Center),
        )
        .padding(22)
        .width(Length::Fill)
        .style(move |_| card(24.0, palette));

        column![mode_buttons, timer_card, controls, stats]
            .spacing(18)
            .width(Length::Fill)
            .into()
    }

    fn settings_view(&self, palette: Palette) -> Element<'_, Message> {
        let settings = self.snapshot.settings;
        let intro = column![
            text("Settings")
                .size(30)
                .font(display_bold())
                .color(palette.text_primary),
            text("Shape your focus routine.")
                .size(15)
                .color(palette.text_secondary),
        ]
        .spacing(7);

        let durations = container(
            column![
                text("Session lengths")
                    .size(18)
                    .font(semibold())
                    .color(palette.text_primary),
                duration_row(Mode::Focus, settings.focus, palette),
                duration_row(Mode::ShortBreak, settings.short_break, palette),
                duration_row(Mode::LongBreak, settings.long_break, palette),
            ]
            .spacing(20),
        )
        .padding(22)
        .width(Length::Fill)
        .style(move |_| card(26.0, palette));

        let focus_sounds = container(
            column![
                focus_sound_row(
                    [FocusSound::Off, FocusSound::Brown, FocusSound::Pink],
                    settings.focus_sound,
                    palette,
                ),
                focus_sound_row(
                    [FocusSound::White, FocusSound::Blue, FocusSound::Violet],
                    settings.focus_sound,
                    palette,
                ),
            ]
            .spacing(4),
        )
        .padding(5)
        .width(Length::Fill)
        .style(move |_| panel(palette.panel_alt, 20.0));

        let finish_sounds = container(
            column![
                finish_sound_row(
                    [FinishSound::Off, FinishSound::Gong, FinishSound::Chime],
                    settings.finish_sound,
                    palette,
                ),
                finish_sound_row(
                    [FinishSound::Bell, FinishSound::Wood, FinishSound::Pulse],
                    settings.finish_sound,
                    palette,
                ),
            ]
            .spacing(4),
        )
        .padding(5)
        .width(Length::Fill)
        .style(move |_| panel(palette.panel_alt, 20.0));

        let sounds = container(
            column![
                text("Sound")
                    .size(18)
                    .font(semibold())
                    .color(palette.text_primary),
                text("Focus ambience")
                    .size(14)
                    .color(palette.text_secondary),
                focus_sounds,
                text("Completion signal")
                    .size(14)
                    .color(palette.text_secondary),
                finish_sounds,
                text("Breaks are always silent.")
                    .size(12)
                    .color(palette.text_secondary),
            ]
            .spacing(12),
        )
        .padding(22)
        .width(Length::Fill)
        .style(move |_| card(26.0, palette));

        let tip = container(
            column![
                text("How the cycle works")
                    .size(16)
                    .font(semibold())
                    .color(palette.text_primary),
                text("Each completed focus session is followed by a break. After the fourth, take a long break.")
                    .size(14)
                    .color(palette.text_secondary),
                text("Changes save automatically. An active session keeps its current duration.")
                    .size(13)
                    .color(palette.text_secondary),
            ]
            .spacing(11),
        )
        .padding(20)
        .width(Length::Fill)
        .style(move |_| panel(palette.panel_alt, 24.0));

        column![intro, durations, sounds, tip]
            .spacing(22)
            .width(Length::Fill)
            .into()
    }
}

fn sound_button(
    label: &'static str,
    selected: bool,
    command: Command,
    palette: Palette,
) -> iced::widget::Button<'static, Message> {
    button(
        container(text(label).size(13).font(if selected {
            semibold()
        } else {
            Font::with_name("Inter")
        }))
        .width(Length::Fill)
        .center_x(Length::Fill),
    )
    .on_press(Message::Notify(command))
    .padding([11, 5])
    .width(Length::Fill)
    .style(move |_, status| segment_button(selected, status, palette))
}

fn focus_sound_row(
    sounds: [FocusSound; 3],
    selected: FocusSound,
    palette: Palette,
) -> iced::widget::Row<'static, Message> {
    sounds
        .into_iter()
        .fold(row![].spacing(4).width(Length::Fill), |row, sound| {
            row.push(sound_button(
                sound.label(),
                sound == selected,
                Command::SetFocusSound(sound),
                palette,
            ))
        })
}

fn finish_sound_row(
    sounds: [FinishSound; 3],
    selected: FinishSound,
    palette: Palette,
) -> iced::widget::Row<'static, Message> {
    sounds
        .into_iter()
        .fold(row![].spacing(4).width(Length::Fill), |row, sound| {
            row.push(sound_button(
                sound.label(),
                sound == selected,
                Command::SetFinishSound(sound),
                palette,
            ))
        })
}

fn mode_button(
    mode: Mode,
    selected: Mode,
    palette: Palette,
) -> iced::widget::Button<'static, Message> {
    button(
        container(text(mode.label()).size(14).font(if mode == selected {
            semibold()
        } else {
            Font::default()
        }))
        .width(Length::Fill)
        .center_x(Length::Fill),
    )
    .on_press(Message::Notify(Command::SelectMode(mode)))
    .padding([12, 8])
    .width(Length::Fill)
    .style(move |_, status| segment_button(mode == selected, status, palette))
}

fn duration_row(mode: Mode, minutes: u32, palette: Palette) -> Element<'static, Message> {
    let max_minutes = if mode == Mode::Focus { 120 } else { 60 };
    row![
        column![
            text(mode.label())
                .size(16)
                .font(semibold())
                .color(palette.text_primary),
            text("minutes").size(12).color(palette.text_secondary),
        ]
        .spacing(2)
        .width(Length::Fill),
        duration_step_button(mode, -5, minutes > 5, palette),
        text(minutes.to_string())
            .size(20)
            .font(semibold())
            .width(40)
            .align_x(Alignment::Center)
            .color(palette.text_primary),
        duration_step_button(mode, 5, minutes < max_minutes, palette),
    ]
    .spacing(13)
    .align_y(Alignment::Center)
    .into()
}

fn duration_step_button(
    mode: Mode,
    delta: i32,
    enabled: bool,
    palette: Palette,
) -> iced::widget::Button<'static, Message> {
    button(
        container(
            text(if delta < 0 { "−5" } else { "+5" })
                .size(17)
                .font(semibold()),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill),
    )
    .on_press_maybe(enabled.then_some(Message::Notify(Command::Adjust(mode, delta))))
    .padding(0)
    .width(48)
    .height(42)
    .style(move |_, status| soft_button(status, palette))
}

fn cycle_indicator(completed: u8, palette: Palette) -> Element<'static, Message> {
    let mut dots = row![].spacing(8);
    for index in 0..4 {
        let filled = index < completed;
        dots = dots.push(
            container(Space::new().width(14).height(14))
                .width(14)
                .height(14)
                .style(move |_| container::Style {
                    background: filled.then(|| palette.accent.into()),
                    border: Border {
                        color: palette.accent,
                        width: 1.5,
                        radius: 7.0.into(),
                    },
                    ..container::Style::default()
                }),
        );
    }
    dots.into()
}

fn panel(color: Color, radius: f32) -> container::Style {
    container::Style {
        background: Some(Background::Color(color)),
        border: Border {
            radius: radius.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

fn card(radius: f32, palette: Palette) -> container::Style {
    container::Style {
        background: Some(palette.surface.into()),
        border: Border {
            color: palette.track,
            width: 1.0,
            radius: radius.into(),
        },
        ..container::Style::default()
    }
}

fn backdrop(palette: Palette) -> container::Style {
    panel(palette.background, 0.0)
}

fn colored_button(
    background: Color,
    foreground: Color,
    status: button::Status,
    radius: f32,
    palette: Palette,
) -> button::Style {
    let background = if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        blend(background, palette.text_primary, 0.08)
    } else {
        background
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: foreground,
        border: Border {
            radius: radius.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

fn primary_button(status: button::Status, palette: Palette) -> button::Style {
    colored_button(palette.accent, palette.on_accent, status, 18.0, palette)
}

fn soft_button(status: button::Status, palette: Palette) -> button::Style {
    if status == button::Status::Disabled {
        return colored_button(
            palette.secondary_surface,
            palette.text_secondary,
            status,
            17.0,
            palette,
        );
    }
    colored_button(
        palette.secondary_surface,
        palette.accent,
        status,
        17.0,
        palette,
    )
}

fn segment_button(selected: bool, status: button::Status, palette: Palette) -> button::Style {
    let background = if selected {
        palette.surface
    } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        blend(palette.panel_alt, palette.surface, 0.55)
    } else {
        Color::TRANSPARENT
    };
    button::Style {
        background: Some(background.into()),
        text_color: if selected {
            palette.text_primary
        } else {
            palette.text_secondary
        },
        border: Border {
            radius: 16.0.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

fn blend(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgb(
        a.r * (1.0 - t) + b.r * t,
        a.g * (1.0 - t) + b.g * t,
        a.b * (1.0 - t) + b.b * t,
    )
}

fn semibold() -> Font {
    Font {
        weight: iced::font::Weight::Semibold,
        ..Font::with_name("Inter")
    }
}

fn display_bold() -> Font {
    Font {
        weight: iced::font::Weight::Bold,
        ..Font::with_name("Inter Display")
    }
}

#[derive(Clone, Copy)]
struct Palette {
    background: Color,
    panel_alt: Color,
    track: Color,
    accent: Color,
    text_primary: Color,
    text_secondary: Color,
    surface: Color,
    secondary_surface: Color,
    on_accent: Color,
}

impl Palette {
    fn for_mode(mode: theme::Mode) -> Self {
        if mode == theme::Mode::Dark {
            Self {
                background: Color::from_rgb8(0x0B, 0x0D, 0x14),
                panel_alt: Color::from_rgb8(0x25, 0x29, 0x35),
                track: Color::from_rgb8(0x35, 0x39, 0x46),
                accent: Color::from_rgb8(0x0A, 0x84, 0xFF),
                text_primary: Color::from_rgb8(0xF5, 0xF7, 0xFD),
                text_secondary: Color::from_rgb8(0xA4, 0xAD, 0xBF),
                surface: Color::from_rgb8(0x1A, 0x1E, 0x29),
                secondary_surface: Color::from_rgb8(0x25, 0x29, 0x35),
                on_accent: Color::WHITE,
            }
        } else {
            Self {
                background: Color::from_rgb8(0xF1, 0xF4, 0xFC),
                panel_alt: Color::from_rgb8(0xE7, 0xEA, 0xF3),
                track: Color::from_rgb8(0xE7, 0xEB, 0xF5),
                accent: Color::from_rgb8(0x00, 0x7A, 0xFF),
                text_primary: Color::from_rgb8(0x1C, 0x23, 0x35),
                text_secondary: Color::from_rgb8(0x79, 0x82, 0x99),
                surface: Color::WHITE,
                secondary_surface: Color::from_rgb8(0xFB, 0xFC, 0xFF),
                on_accent: Color::WHITE,
            }
        }
    }
}
