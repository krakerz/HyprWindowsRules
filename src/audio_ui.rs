//! The "Audio routes" tab.

use std::time::Duration;

use iced::widget::{
    Column, button, center, checkbox, column, container, pick_list, row, scrollable, space, text,
    text_input,
};
use iced::{Element, Fill, Font, Task};

use crate::audio::{self, Device, DeviceInfo, Route, Snapshot, Status};

#[derive(Debug, Clone)]
pub enum Msg {
    Refresh,
    Refreshed(Result<Snapshot, String>),
    Select(usize),
    Add,
    Delete,
    Name(String),
    Enabled(bool),
    ProgramInput(String),
    AddProgram,
    PickProgram(String),
    RemoveProgram(usize),
    Output(Choice),
    Input(Choice),
    OutputPattern(String),
    InputPattern(String),
    Restart,
    RestartConfirmed,
    CancelRestart,
    TurnOff,
    TurnOn,
}

/// A device picker entry: a connected device, or "leave it alone".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice(Option<DeviceInfo>);

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(d) => write!(f, "{d}"),
            None => f.write_str("Don't change"),
        }
    }
}

pub struct AudioTab {
    routes: Vec<Route>,
    selected: Option<usize>,
    pub dirty: bool,
    snap: Snapshot,
    status: Status,
    note: String,
    program_input: String,
    confirm_restart: bool,
}

impl AudioTab {
    /// Placeholder until boot replaces it with `new()`'s result.
    pub fn default_closed() -> Self {
        Self {
            routes: Vec::new(),
            selected: None,
            dirty: false,
            snap: Snapshot::default(),
            status: Status::NotInstalled,
            note: String::new(),
            program_input: String::new(),
            confirm_restart: false,
        }
    }

    pub fn new() -> (Self, Task<Msg>) {
        let (routes, note) = match audio::load_routes() {
            Ok(r) => (r, String::new()),
            Err(e) => (Vec::new(), e),
        };
        let tab = Self {
            selected: if routes.is_empty() { None } else { Some(0) },
            routes,
            dirty: false,
            snap: Snapshot::default(),
            status: Status::NotInstalled,
            note,
            program_input: String::new(),
            confirm_restart: false,
        };
        (tab, Task::done(Msg::Refresh))
    }

    fn current(&mut self) -> Option<&mut Route> {
        self.selected.and_then(|i| self.routes.get_mut(i))
    }

    fn edit(&mut self, f: impl FnOnce(&mut Route)) {
        if let Some(r) = self.current() {
            f(r);
            self.dirty = true;
        }
    }

    fn refresh(after: Duration) -> Task<Msg> {
        Task::perform(
            async move {
                tokio::time::sleep(after).await;
                audio::snapshot()
            },
            Msg::Refreshed,
        )
    }

    pub fn save(&mut self) -> Task<Msg> {
        if self.status == Status::Off {
            return match audio::save_routes(&self.routes) {
                Ok(()) => {
                    self.dirty = false;
                    self.note = "Saved. Routing is off — Turn on to apply it.".into();
                    Task::none()
                }
                Err(e) => {
                    self.note = format!("Couldn't save: {e}");
                    Task::none()
                }
            };
        }
        match audio::install(&self.routes) {
            Ok(script_changed) => {
                self.dirty = false;
                self.note = if self.status == Status::Active && !script_changed {
                    match audio::apply_live(&self.routes) {
                        Ok(()) => "Saved — running programs were moved to their routes.".into(),
                        Err(e) => format!("Saved, but applying it live failed: {e}"),
                    }
                } else {
                    "Saved. WirePlumber needs a restart to start routing.".into()
                };
                Self::refresh(Duration::ZERO)
            }
            Err(e) => {
                self.note = format!("Couldn't save: {e}");
                Task::none()
            }
        }
    }

    pub fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Refresh => return Self::refresh(Duration::ZERO),
            Msg::Refreshed(Ok(snap)) => {
                self.status = audio::status(&snap);
                self.snap = snap;
            }
            Msg::Refreshed(Err(e)) => self.note = format!("Can't read PipeWire: {e}"),
            Msg::Select(i) => self.selected = Some(i),
            Msg::Add => {
                self.routes.push(Route::default());
                self.selected = Some(self.routes.len() - 1);
                self.dirty = true;
            }
            Msg::Delete => {
                if let Some(i) = self.selected {
                    self.routes.remove(i);
                    self.selected = if self.routes.is_empty() {
                        None
                    } else {
                        Some(i.min(self.routes.len() - 1))
                    };
                    self.dirty = true;
                }
            }
            Msg::Name(v) => self.edit(|r| r.name = v),
            Msg::Enabled(v) => self.edit(|r| r.enabled = v),
            Msg::ProgramInput(v) => self.program_input = v,
            Msg::AddProgram => {
                let p = self.program_input.trim().to_string();
                if !p.is_empty() {
                    self.edit(|r| {
                        if !r.programs.contains(&p) {
                            r.programs.push(p)
                        }
                    });
                    self.program_input.clear();
                }
            }
            Msg::PickProgram(p) => self.edit(|r| {
                if !r.programs.contains(&p) {
                    r.programs.push(p)
                }
            }),
            Msg::RemoveProgram(i) => self.edit(|r| {
                r.programs.remove(i);
            }),
            Msg::Output(c) => self.edit(|r| r.output = c.0.map(device_from)),
            Msg::Input(c) => self.edit(|r| r.input = c.0.map(device_from)),
            Msg::OutputPattern(p) => self.edit(|r| set_pattern(&mut r.output, p)),
            Msg::InputPattern(p) => self.edit(|r| set_pattern(&mut r.input, p)),
            Msg::Restart => self.confirm_restart = true,
            Msg::CancelRestart => self.confirm_restart = false,
            Msg::RestartConfirmed => {
                self.confirm_restart = false;
                self.note = match audio::restart_wireplumber() {
                    Ok(()) => "WirePlumber restarted.".into(),
                    Err(e) => format!("Couldn't restart WirePlumber: {e}"),
                };
                // Give WirePlumber a moment to come back before checking on it
                return Self::refresh(Duration::from_secs(2));
            }
            Msg::TurnOff => {
                // Empty the live routes first so routing stops now, without a restart
                let _ = audio::apply_live(&[]);
                self.note = match audio::save_routes(&self.routes).and_then(|_| audio::uninstall())
                {
                    Ok(()) => {
                        "Audio routing is off. Your routes are kept — Turn on to use them again."
                            .into()
                    }
                    Err(e) => format!("Couldn't turn routing off: {e}"),
                };
                return Self::refresh(Duration::ZERO);
            }
            Msg::TurnOn => {
                self.note = match audio::install(&self.routes) {
                    Ok(_) => {
                        self.dirty = false;
                        // WirePlumber may still run the script from before; hand it the routes
                        match audio::apply_live(&self.routes) {
                            Ok(()) => "Audio routing is on again.".into(),
                            Err(_) => "Routing files installed.".into(),
                        }
                    }
                    Err(e) => format!("Couldn't turn routing on: {e}"),
                };
                return Self::refresh(Duration::ZERO);
            }
        }
        Task::none()
    }

    // ---- view ---------------------------------------------------------------------

    pub fn toolbar(&self) -> Element<'_, Msg> {
        let tool = |label: &'static str, msg: Option<Msg>| {
            button(text(label).size(14))
                .on_press_maybe(msg)
                .style(button::secondary)
        };
        row![
            tool("Add route", Some(Msg::Add)),
            tool("Delete", self.selected.map(|_| Msg::Delete)),
            space().width(12),
            tool("Refresh devices", Some(Msg::Refresh)),
        ]
        .spacing(6)
        .into()
    }

    pub fn view(&self) -> Element<'_, Msg> {
        column![
            self.banner(),
            row![container(self.view_list()).width(380), self.view_editor()]
                .spacing(8)
                .height(Fill)
        ]
        .spacing(8)
        .into()
    }

    fn banner(&self) -> Element<'_, Msg> {
        let small = |s: &str| text(s.to_string()).size(13);
        let mut r = row![].spacing(8).align_y(iced::Center);
        if self.confirm_restart {
            r = r
                .push(
                    small("Restarting WirePlumber interrupts all audio for about a second.")
                        .width(Fill),
                )
                .push(
                    button(text("Restart").size(13))
                        .on_press(Msg::RestartConfirmed)
                        .style(button::danger),
                )
                .push(
                    button(text("Cancel").size(13))
                        .on_press(Msg::CancelRestart)
                        .style(button::secondary),
                );
        } else {
            let (msg, action): (&str, Option<(&str, Msg)>) = match self.status {
                Status::NotInstalled => (
                    "Audio routing isn't set up. Saving installs a small WirePlumber script that moves each program's sound to its device — it keeps working when this app is closed.",
                    None,
                ),
                Status::Off => (
                    "Audio routing is turned off. Your routes are kept; turning it on reinstalls the WirePlumber script.",
                    Some(("Turn on", Msg::TurnOn)),
                ),
                Status::NeedsRestart => (
                    "WirePlumber needs a restart to load the routing script.",
                    Some(("Restart WirePlumber", Msg::Restart)),
                ),
                Status::Active => (
                    "Routing is active — saved changes apply to running programs right away.",
                    Some(("Turn off", Msg::TurnOff)),
                ),
            };
            r = r.push(small(msg).width(Fill));
            if let Some((label, m)) = action {
                r = r.push(
                    button(text(label).size(13))
                        .on_press(m)
                        .style(button::secondary),
                );
            }
        }
        let mut col = column![r].spacing(4);
        if !self.note.is_empty() {
            col = col.push(text(self.note.as_str()).size(13).style(text::secondary));
        }
        container(col)
            .padding(10)
            .width(Fill)
            .style(container::bordered_box)
            .into()
    }

    fn view_list(&self) -> Element<'_, Msg> {
        let mut items = Column::new().spacing(2);
        for (i, r) in self.routes.iter().enumerate() {
            let off = if r.enabled { "" } else { "   (off)" };
            let label = column![
                text(format!("{}{off}", r.title())).size(14),
                text(r.summary()).size(12).style(text::secondary),
            ]
            .spacing(2);
            let style = if self.selected == Some(i) {
                button::primary
            } else {
                button::text
            };
            items = items.push(
                button(label)
                    .width(Fill)
                    .on_press(Msg::Select(i))
                    .style(style),
            );
        }
        scrollable(items.padding(iced::Padding::ZERO.right(12)))
            .height(Fill)
            .into()
    }

    fn view_editor(&self) -> Element<'_, Msg> {
        let Some(route) = self.selected.and_then(|i| self.routes.get(i)) else {
            return center(
                text("Add a route to send a program's sound to a specific device.")
                    .style(text::secondary),
            )
            .into();
        };
        let label = |s: &'static str| text(s).width(110);

        let mut programs = row![].spacing(6).align_y(iced::Center);
        for (i, p) in route.programs.iter().enumerate() {
            programs = programs.push(
                button(text(format!("{p}  ✕")).size(13))
                    .on_press(Msg::RemoveProgram(i))
                    .style(button::secondary),
            );
        }
        let available: Vec<String> = self
            .snap
            .programs
            .iter()
            .filter(|p| !route.programs.contains(p))
            .cloned()
            .collect();

        let device_row = |title: &'static str,
                          devices: &[DeviceInfo],
                          current: &Option<Device>,
                          on_pick: fn(Choice) -> Msg,
                          on_pattern: fn(String) -> Msg| {
            let mut options = vec![Choice(None)];
            options.extend(devices.iter().cloned().map(|d| Choice(Some(d))));
            let selected = match current {
                None => Some(Choice(None)),
                Some(dev) => devices
                    .iter()
                    .find(|d| matches(dev, d))
                    .cloned()
                    .map(|d| Choice(Some(d))),
            };
            let mut col = column![
                row![
                    label(title),
                    pick_list(options, selected, on_pick)
                        .placeholder(
                            current
                                .as_ref()
                                .map(|d| format!("{} (not connected)", d.label()))
                                .unwrap_or_default()
                        )
                        .width(360),
                ]
                .spacing(8)
                .align_y(iced::Center)
            ]
            .spacing(6);
            if let Some(dev) = current {
                col = col.push(
                    row![
                        space().width(110),
                        text_input(
                            "also match device names like alsa_output.usb-Brand_*-01.pro-output-0",
                            dev.pattern.as_deref().unwrap_or("")
                        )
                        .on_input(on_pattern)
                        .size(13)
                        .width(Fill),
                    ]
                    .spacing(8),
                );
            }
            col
        };

        let body = column![
            row![label("Name"), text_input("e.g. Chat apps", &route.name).on_input(Msg::Name)]
                .spacing(8)
                .align_y(iced::Center),
            row![space().width(110), checkbox(route.enabled).label("Route is active").on_toggle(Msg::Enabled)].spacing(8),
            heading("Programs"),
            container(
                column![
                    if route.programs.is_empty() {
                        Element::from(text("No programs yet — add the ones this route applies to.").size(13).style(text::secondary))
                    } else {
                        programs.wrap().into()
                    },
                    row![
                        pick_list(available, None::<String>, Msg::PickProgram)
                            .placeholder("Playing or recording now…")
                            .width(260),
                        text_input("or type a program name, e.g. Discord", &self.program_input)
                            .on_input(Msg::ProgramInput)
                            .on_submit(Msg::AddProgram)
                            .width(Fill),
                        button(text("Add").size(14)).on_press(Msg::AddProgram).style(button::secondary),
                    ]
                    .spacing(6),
                    text("Matches the program's executable name, however it connects to audio (PulseAudio or ALSA).")
                        .size(13)
                        .style(text::secondary),
                ]
                .spacing(8),
            )
            .padding(12)
            .width(Fill)
            .style(container::bordered_box),
            heading("Devices"),
            container(
                column![
                    device_row("Output", &self.snap.outputs, &route.output, Msg::Output, Msg::OutputPattern),
                    device_row("Input", &self.snap.inputs, &route.input, Msg::Input, Msg::InputPattern),
                    text("A device matches by its name as shown here, or by the device-name pattern — handy when a device's name changes between connection modes. When it's unplugged the program uses the default device, and moves back when it returns.")
                        .size(13)
                        .style(text::secondary),
                ]
                .spacing(10),
            )
            .padding(12)
            .width(Fill)
            .style(container::bordered_box),
        ]
        .spacing(12)
        .padding(iced::Padding::ZERO.right(16).bottom(16));
        scrollable(body).height(Fill).into()
    }
}

fn heading(s: &str) -> iced::widget::Text<'_> {
    text(s).size(15).font(Font {
        weight: iced::font::Weight::Bold,
        ..Font::DEFAULT
    })
}

fn device_from(d: DeviceInfo) -> Device {
    Device {
        description: Some(d.description),
        name: Some(d.name),
        pattern: None,
    }
}

fn set_pattern(dev: &mut Option<Device>, pattern: String) {
    if let Some(d) = dev {
        d.pattern = if pattern.trim().is_empty() {
            None
        } else {
            Some(pattern)
        };
    }
}

/// Mirrors the script's device_matches for the description and exact name.
fn matches(dev: &Device, info: &DeviceInfo) -> bool {
    dev.description.as_deref() == Some(info.description.as_str())
        || dev.name.as_deref() == Some(info.name.as_str())
}
