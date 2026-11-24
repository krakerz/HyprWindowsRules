//! Modal dialogs and their handlers.

use iced::widget::{
    Button, Column, button, checkbox, column, container, row, scrollable, space, text,
};
use iced::{Element, Fill, Length, Task};
use serde_json::Value as Json;

use super::style::{
    BOLD, CONTROL_H, GAP, PAD, SECTION_GAP, TEXT, TEXT_SMALL, TEXT_TITLE, U, action, single_line,
};
use super::{App, Geometry, Message, Modal};
use crate::model::{MatchField, Rule, Value};
use crate::settings;
use crate::update::{self, InstallKind};
use crate::{hypr, luaio};

impl App {
    pub(super) fn picker(
        &mut self,
        f: impl FnOnce(
            (
                &mut Vec<Json>,
                &mut Option<usize>,
                &mut bool,
                &mut bool,
                &mut bool,
            ),
        ),
    ) {
        if let Some(Modal::Picker {
            clients,
            selected,
            use_title,
            copy_geom,
            copy_ws,
            ..
        }) = &mut self.modal
        {
            f((clients, selected, use_title, copy_geom, copy_ws));
        }
    }

    pub(super) fn picker_ok(&mut self) {
        let Some(Modal::Picker {
            clients,
            selected: Some(i),
            use_title,
            copy_geom,
            copy_ws,
            for_current,
        }) = self.modal.take()
        else {
            return;
        };
        let Some(c) = clients.get(i) else { return };
        if let Some(part) = for_current {
            let (w, h) = hypr::size(c);
            let (x, y) = hypr::relative_position(c, &self.monitors);
            self.edit(|e| match part {
                Geometry::Size => (e.size_w, e.size_h) = (w.to_string(), h.to_string()),
                Geometry::Position => (e.move_x, e.move_y) = (x.to_string(), y.to_string()),
            });
            return;
        }
        let class = hypr::str_field(c, "class");
        let mut matches = vec![MatchField::exact("class", class)];
        if use_title {
            matches.push(MatchField::exact("title", hypr::str_field(c, "title")));
        }
        let mut effects = Vec::new();
        let floating = hypr::floating(c);
        if floating {
            effects.push(("float".into(), Value::Bool(true)));
        }
        if copy_geom {
            let (w, h) = hypr::size(c);
            effects.push((
                "size".into(),
                Value::List(vec![Value::Str(w.to_string()), Value::Str(h.to_string())]),
            ));
            if floating {
                let (x, y) = hypr::relative_position(c, &self.monitors);
                effects.push((
                    "move".into(),
                    Value::List(vec![Value::Str(x.to_string()), Value::Str(y.to_string())]),
                ));
            }
        }
        if copy_ws {
            effects.push((
                "workspace".into(),
                Value::Str(hypr::workspace_name(c).into()),
            ));
        }
        self.insert(Rule {
            name: class.into(),
            matches,
            effects,
            ..Default::default()
        });
    }

    pub(super) fn import_ok(&mut self) -> Task<Message> {
        let Some(Modal::Import {
            path,
            rules,
            checked,
            comment_out,
            ..
        }) = self.modal.take()
        else {
            return Task::none();
        };
        let chosen: Vec<usize> = (0..rules.len()).filter(|&i| checked[i]).collect();
        if chosen.is_empty() {
            return Task::none();
        }
        self.rules.extend(chosen.iter().map(|&i| rules[i].clone()));
        self.dirty = true;
        self.search.clear();
        self.select(Some(self.rules.len() - 1));
        if !comment_out {
            return Task::none();
        }
        // Save first, then comment out: the rules must never be missing from both files
        if let Err(err) = self.write_file(&self.settings.rules()) {
            self.info("Save failed", err.to_string());
            return Task::none();
        }
        self.dirty = false;
        self.saved_rules = self.rules.clone();
        self.select(self.selected);
        if !settings::hook_installed(&self.settings.hook(), &self.settings.rules()) {
            self.info(
                "Import",
                "Rules imported and saved, but the originals were left in place because Hyprland isn't set up to load the rules file yet.",
            );
            return Task::none();
        }
        match luaio::comment_out_calls(&path, &chosen) {
            Ok(backup) => {
                let name = backup
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.note = format!(
                    "Imported {} rule(s); originals commented out (backup: {name})",
                    chosen.len()
                );
                self.reload_hyprland()
            }
            Err(err) => {
                self.info(
                    "Import",
                    format!("Rules imported, but commenting out the originals failed:\n\n{err}"),
                );
                Task::none()
            }
        }
    }

    pub(super) fn settings_dlg(&mut self, f: impl FnOnce(&mut String, &mut String, &mut bool)) {
        if let Some(Modal::Settings {
            rules,
            hook,
            reload,
            ..
        }) = &mut self.modal
        {
            f(rules, hook, reload);
        }
    }

    pub(super) fn settings_ok(&mut self) -> Task<Message> {
        let Some(Modal::Settings {
            rules,
            hook,
            reload,
            check_updates,
            ..
        }) = self.modal.take()
        else {
            return Task::none();
        };
        let old_rules = self.settings.rules();
        self.settings.rules_path = if rules.trim().is_empty() {
            settings::default_rules().to_string_lossy().into_owned()
        } else {
            rules.trim().into()
        };
        self.settings.hook_file = if hook.trim().is_empty() {
            settings::default_hook().to_string_lossy().into_owned()
        } else {
            hook.trim().into()
        };
        self.settings.reload_on_save = reload;
        self.settings.check_updates = check_updates;
        if let Err(err) = self.settings.save() {
            self.info("Settings", format!("Couldn't save settings:\n\n{err}"));
        }
        if self.settings.rules() != old_rules {
            if self.dirty {
                self.modal = Some(Modal::Message {
                    title: "Rules file changed".into(),
                    body: "Save the current rules to the new file, or load what's in the new file?"
                        .into(),
                    buttons: vec![
                        ("Save here".into(), Message::SaveRules),
                        ("Load file".into(), Message::LoadRules),
                    ],
                });
            } else {
                self.load_rules();
            }
        }
        Task::none()
    }

    pub(super) fn view_modal<'a>(&'a self, modal: &'a Modal) -> Element<'a, Message> {
        match modal {
            Modal::Message {
                title,
                body,
                buttons,
            } => {
                // The first button is the main action; it goes right-most
                let btns = buttons
                    .iter()
                    .enumerate()
                    .rev()
                    .map(|(i, (label, msg))| {
                        let b = control_btn(label.as_str()).on_press(msg.clone());
                        if i == 0 { b.style(button::primary) } else { b }.into()
                    })
                    .collect();
                chrome(title, W_S, text(body.as_str()).size(TEXT).into(), btns)
            }
            Modal::Picker {
                clients,
                selected,
                use_title,
                copy_geom,
                copy_ws,
                for_current,
            } => {
                let mut list = Column::new().spacing(U / 2.0);
                for (i, c) in clients.iter().enumerate() {
                    let cells = row![
                        clipped(hypr::str_field(c, "class"), TEXT, 3),
                        clipped(hypr::str_field(c, "title"), TEXT, 5),
                        clipped(hypr::workspace_name(c), TEXT, 2),
                        clipped(if hypr::floating(c) { "floating" } else { "" }, TEXT, 1),
                    ]
                    .spacing(GAP)
                    .height(Fill)
                    .align_y(iced::Center);
                    let style = if *selected == Some(i) {
                        button::primary
                    } else {
                        button::text
                    };
                    list = list.push(
                        button(cells)
                            .width(Fill)
                            .height(CONTROL_H)
                            .padding([0.0, PAD])
                            .on_press(Message::PickerSelect(i))
                            .style(style),
                    );
                }
                let title = match for_current {
                    Some(Geometry::Size) => "Copy the size of an open window",
                    Some(Geometry::Position) => "Copy the position of an open window",
                    None => "Create rule from an open window",
                };
                let head = |s: &'static str, portion: u16| {
                    container(single_line(s, TEXT_SMALL).style(text::secondary))
                        .width(Length::FillPortion(portion))
                        .clip(true)
                };
                let mut col = column![
                    row![
                        head("Class", 3),
                        head("Title", 5),
                        head("Workspace", 2),
                        head("", 1)
                    ]
                    .spacing(GAP)
                    .padding([0.0, PAD]),
                    scrollable(list).height(10.0 * CONTROL_H),
                ]
                .spacing(GAP);
                if let Some(part) = for_current {
                    let hint = match part {
                        Geometry::Size => "Its current size replaces the rule's Size.",
                        Geometry::Position => {
                            "Its current position (relative to its monitor) replaces the rule's Position."
                        }
                    };
                    col = col.push(text(hint).size(TEXT_SMALL).style(text::secondary));
                } else {
                    col = col.extend([
                        checkbox(*use_title)
                            .label("Also match the window title (only this exact window, not every window of the app)")
                            .text_size(TEXT)
                            .on_toggle(Message::PickerUseTitle)
                            .into(),
                        checkbox(*copy_geom)
                            .label("Copy its current size and position")
                            .text_size(TEXT)
                            .on_toggle(Message::PickerCopyGeom)
                            .into(),
                        checkbox(*copy_ws)
                            .label("Copy its current workspace")
                            .text_size(TEXT)
                            .on_toggle(Message::PickerCopyWs)
                            .into(),
                    ]);
                }
                chrome(
                    title,
                    W_L,
                    col.into(),
                    ok_cancel_buttons(selected.map(|_| Message::PickerOk)),
                )
            }
            Modal::Import {
                path,
                rules,
                checked,
                can_comment,
                comment_out,
                warning,
            } => {
                let mut list = Column::new().spacing(GAP);
                for (i, r) in rules.iter().enumerate() {
                    list = list.push(
                        container(
                            checkbox(checked[i])
                                .label(format!("{}   →   {}", r.title(), r.summary()))
                                .text_size(TEXT)
                                .text_wrapping(text::Wrapping::None)
                                .on_toggle(move |v| Message::ImportToggle(i, v)),
                        )
                        .width(Fill)
                        .clip(true),
                    );
                }
                let mut col = column![
                    text(format!(
                        "Found {} window rule(s) in {}. Choose which to import:",
                        rules.len(),
                        path.display()
                    ))
                    .size(TEXT),
                ]
                .spacing(GAP);
                if let Some(w) = warning {
                    col = col.push(text(w.as_str()).size(TEXT).style(text::danger));
                }
                col = col.push(scrollable(list).height(8.0 * CONTROL_H));
                let cb = checkbox(*comment_out)
                    .label("Comment out the imported rules in that file, so they aren't applied twice (a backup is kept)")
                    .text_size(TEXT)
                    .on_toggle_maybe(can_comment.then_some(Message::ImportCommentOut));
                col = col.push(cb);
                if !can_comment {
                    col = col.push(
                        text("The rules in this file couldn't be located reliably in its text, so they'll be left in place.")
                            .size(TEXT_SMALL)
                            .style(text::secondary),
                    );
                }
                let title = format!(
                    "Import rules from {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                chrome(
                    title,
                    W_M,
                    col.into(),
                    ok_cancel_buttons(checked.iter().any(|c| *c).then_some(Message::ImportOk)),
                )
            }
            Modal::Inspector => self.view_inspector(),
            Modal::History => self.view_history(),
            Modal::Settings {
                rules,
                hook,
                reload,
                check_updates,
                ..
            } => {
                // Empty means the default
                let problem = (!rules.trim().is_empty())
                    .then(|| settings::rules_path_problem(&settings::expand(rules)))
                    .flatten();
                use super::style::{self, GAP, LABEL_W};
                use crate::settings::ThemePref;
                use iced::widget::{container, pick_list};
                let can_update = self.install_kind != InstallKind::Source;
                let browse = |msg: Message| control_btn("Browse…").on_press(msg);
                let help = |s: &'static str| {
                    row![
                        space().width(LABEL_W),
                        text(s)
                            .size(style::TEXT_SMALL)
                            .style(text::secondary)
                            .width(Fill)
                    ]
                    .spacing(GAP)
                };
                let aligned =
                    |c: Element<'a, Message>| row![space().width(LABEL_W), c].spacing(GAP);
                let help_danger = |p: &'static str| {
                    row![
                        space().width(LABEL_W),
                        text(p).size(style::TEXT_SMALL).style(text::danger)
                    ]
                    .spacing(GAP)
                };
                let body = column![
                    style::form_row(
                        "Theme",
                        pick_list(&ThemePref::ALL[..], Some(self.settings.theme), Message::Theme)
                            .text_size(style::TEXT)
                            .padding([0.0, 12.0])
                            .text_line_height(text::LineHeight::Absolute(style::CONTROL_H.into()))
                            .width(Fill),
                    ),
                    style::form_row(
                        "Rules file",
                        row![
                            style::input("", rules).on_input(Message::SettingsRules).width(Fill),
                            browse(Message::BrowseRules),
                        ]
                        .spacing(GAP),
                    ),
                    problem.map(help_danger),
                    style::form_row(
                        "Load it from",
                        row![
                            style::input("", hook).on_input(Message::SettingsHook).width(Fill),
                            browse(Message::BrowseHook),
                        ]
                        .spacing(GAP),
                    ),
                    help(
                        "The app adds one line to the “Load it from” file so Hyprland loads the rules file, \
                         and reloads by itself whenever the rules change. Usually ~/.config/hypr/hyprland.lua."
                    ),
                    aligned(
                        checkbox(*reload)
                            .label("Reload Hyprland after saving")
                            .on_toggle(Message::SettingsReload)
                            .into()
                    ),
                    style::form_row(
                        "Updates",
                        row![
                            container(style::single_line(
                                format!(
                                    "Version {} · {}",
                                    update::current_version(),
                                    self.install_kind.describe()
                                ),
                                style::TEXT,
                            ))
                            .width(Fill)
                            .clip(true),
                            control_btn("Check now")
                                .on_press_maybe(can_update.then_some(Message::CheckUpdates)),
                        ]
                        .spacing(GAP)
                        .align_y(iced::Center),
                    ),
                    style::form_row(
                        "Property docs",
                        row![
                            container(style::single_line(self.catalog.status(), style::TEXT))
                                .width(Fill)
                                .clip(true),
                            control_btn("Refresh").on_press_maybe(
                                (!self.docs_busy).then_some(Message::DocsRefresh)
                            ),
                        ]
                        .spacing(GAP)
                        .align_y(iced::Center),
                    ),
                    aligned(
                        checkbox(*check_updates)
                            .label("Check for updates at startup")
                            .on_toggle_maybe(can_update.then_some(Message::SettingsCheckUpdates))
                            .into()
                    ),
                ]
                .spacing(style::GAP);
                chrome(
                    "Settings",
                    W_M,
                    body.into(),
                    ok_cancel_buttons(problem.is_none().then_some(Message::SettingsOk)),
                )
            }
        }
    }
}

// Dialog widths (design pass): small / medium / large.
pub(super) const W_S: f32 = 420.0;
pub(super) const W_M: f32 = 640.0;
pub(super) const W_L: f32 = 960.0;

/// Dialog body under a TEXT_TITLE title, with a right-aligned button row. `buttons` are in
/// display order, so the main action goes last.
pub(super) fn chrome<'a>(
    title: impl text::IntoFragment<'a>,
    width: f32,
    body: Element<'a, Message>,
    buttons: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    column![
        text(title).size(TEXT_TITLE).font(BOLD),
        body,
        row![space().width(Fill)].extend(buttons).spacing(GAP),
    ]
    .spacing(SECTION_GAP)
    .width(width)
    .into()
}

/// CONTROL_H-tall secondary button with a vertically centred label.
pub(super) fn control_btn<'a>(label: impl text::IntoFragment<'a>) -> Button<'a, Message> {
    button(container(single_line(label, TEXT)).center_y(Fill))
        .height(CONTROL_H)
        .padding([0.0, PAD])
        .style(action)
}

/// Cancel + OK; OK is disabled while `ok` is `None`.
fn ok_cancel_buttons<'a>(ok: Option<Message>) -> Vec<Element<'a, Message>> {
    vec![
        control_btn("Cancel").on_press(Message::CloseModal).into(),
        control_btn("OK")
            .on_press_maybe(ok)
            .style(button::primary)
            .into(),
    ]
}

/// One-line text cell sharing a row by `portion`; clipped, never wrapped.
fn clipped<'a>(s: impl text::IntoFragment<'a>, size: f32, portion: u16) -> Element<'a, Message> {
    container(single_line(s, size))
        .width(Length::FillPortion(portion))
        .clip(true)
        .into()
}
