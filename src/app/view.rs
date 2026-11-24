//! Main window layout.

use iced::widget::{
    button, center, column, container, progress_bar, row, space, stack, text, tooltip,
};
use iced::{Element, Fill};

use super::style::{self, CONTROL_H, GAP, PAD, TEXT, TEXT_SMALL, U, overlay, single_line};
use super::{APP_TITLE, App, Message, Tab, UpdateState};
use super::{history, inspect};
use crate::settings;
use crate::update;

impl App {
    pub(super) fn view(&self) -> Element<'_, Message> {
        let tip = |content: Element<'static, Message>, tip: &'static str, pos| {
            tooltip(
                content,
                container(text(tip).size(TEXT_SMALL))
                    .padding(GAP)
                    .style(container::rounded_box),
                pos,
            )
        };
        let tool =
            |label: &'static str, msg: Message, hint: &'static str| -> Element<'_, Message> {
                let b = style::tool_button(label).on_press(msg);
                tip(b.into(), hint, tooltip::Position::Bottom).into()
            };
        let tab_button = |label: &'static str, tab: Tab| -> Element<'_, Message> {
            let active = self.tab == tab;
            let underline = container(container(space()).width(Fill).height(2).style(
                move |theme: &iced::Theme| {
                    if active {
                        style::rule_line(theme)
                    } else {
                        container::Style::default()
                    }
                },
            ))
            .height(Fill)
            .align_y(iced::Bottom);
            stack![
                button(style::centered(style::single_line(label, TEXT)))
                    .height(CONTROL_H)
                    .padding([0.0, 12.0])
                    .on_press(Message::Tab(tab))
                    .style(style::tab(active)),
                underline
            ]
            .into()
        };
        let tabs = row![
            tab_button("Window rules", Tab::Rules),
            tab_button("Audio routes", Tab::Audio)
        ]
        .spacing(U);
        let tab_tools: Element<_> = match self.tab {
            Tab::Rules => row![
                tool(
                    "Inspector",
                    Message::Inspect(inspect::Msg::Open),
                    "Which rules hit which window"
                ),
                tool(
                    "History",
                    Message::History(history::Msg::Open),
                    "Earlier saved versions"
                ),
                style::divider_v(),
                tool(
                    "Import…",
                    Message::Import,
                    "Import window rules from another Hyprland .lua file"
                ),
                tool("Refresh windows", Message::Refresh, "F5"),
            ]
            .spacing(GAP)
            .into(),
            Tab::Audio => self.audio.toolbar().map(Message::Audio),
        };
        let dirty = self.dirty || self.audio.dirty;
        let save = button(style::centered(style::single_line(
            if dirty { "Save •" } else { "Save" },
            TEXT,
        )))
        .height(CONTROL_H)
        .padding([0.0, 12.0])
        .on_press_maybe(dirty.then_some(Message::Save))
        .style(button::primary);
        let gear = tip(
            button(center(text("⚙").size(TEXT)))
                .width(CONTROL_H)
                .height(CONTROL_H)
                .padding(0)
                .on_press(Message::OpenSettings)
                .style(style::action)
                .into(),
            "Settings",
            tooltip::Position::Bottom,
        );
        let toolbar = container(
            row![
                tabs,
                space().width(Fill),
                tab_tools,
                style::divider_v(),
                save,
                gear,
            ]
            .spacing(GAP)
            .align_y(iced::Center),
        )
        .padding(PAD)
        .height(CONTROL_H + 2.0 * PAD)
        .center_y(CONTROL_H + 2.0 * PAD);

        let body: Element<_> = match self.tab {
            Tab::Rules => row![self.view_list(), container(self.view_editor()).width(Fill),]
                .spacing(0)
                .padding(iced::Padding::ZERO.right(PAD))
                .height(Fill)
                .into(),
            Tab::Audio => container(self.audio.view().map(Message::Audio))
                .padding(PAD)
                .height(Fill)
                .into(),
        };

        let hooked = settings::hook_installed(&self.settings.hook(), &self.settings.rules());
        let mut status = format!(
            "{} rule(s) · {}",
            self.rules.len(),
            settings::display_path(&self.settings.rules())
        );
        if self.dirty {
            status.push_str(" · unsaved changes");
        }
        if !self.note.is_empty() {
            status = format!("{status} · {}", self.note);
        }
        let mut bar = row![
            container(style::single_line(status, TEXT_SMALL).style(text::secondary))
                .width(Fill)
                .clip(true)
        ]
        .spacing(GAP)
        .align_y(iced::Center);
        bar = bar.push(self.view_update());
        if !hooked {
            bar = bar.push(tooltip(
                button(style::centered(style::single_line(
                    "Load rules in Hyprland",
                    TEXT_SMALL,
                )))
                .height(CONTROL_H - 8.0)
                .padding([0.0, 12.0])
                .on_press(Message::InstallHook)
                .style(style::action),
                container(
                    text(format!(
                        "Add a line to {} so Hyprland loads {}",
                        settings::display_path(&self.settings.hook()),
                        settings::display_path(&self.settings.rules())
                    ))
                    .size(TEXT_SMALL),
                )
                .padding(GAP)
                .style(container::rounded_box),
                tooltip::Position::Top,
            ));
        }
        let status_bar = container(bar)
            .padding([U, PAD])
            .height(CONTROL_H + 2.0 * U)
            .center_y(CONTROL_H + 2.0 * U);

        let base: Element<_> = column![
            toolbar,
            style::divider_h(),
            body,
            style::divider_h(),
            status_bar
        ]
        .into();
        match &self.modal {
            None => base,
            Some(m) => overlay(base, self.view_modal(m)),
        }
    }

    pub(super) fn view_update(&self) -> Element<'_, Message> {
        let small = |s: String| single_line(s, TEXT_SMALL).style(text::secondary);
        let btn = |label: &'static str| {
            button(style::centered(single_line(label, TEXT_SMALL)))
                .height(CONTROL_H - 8.0)
                .padding([0.0, 12.0])
        };
        match &self.update {
            UpdateState::Idle => space().into(),
            UpdateState::Checking => small("Checking for updates…".into()).into(),
            UpdateState::UpToDate => small(format!(
                "{APP_TITLE} {} is up to date",
                update::current_version()
            ))
            .into(),
            UpdateState::Available(found) => row![
                small(format!("{APP_TITLE} {} is available", found.version)),
                btn("Update")
                    .on_press(Message::InstallUpdate)
                    .style(button::primary),
            ]
            .spacing(GAP)
            .align_y(iced::Center)
            .into(),
            UpdateState::Downloading { version, progress } => row![
                small(format!("Downloading {version} — {}", progress.describe())),
                progress_bar(0.0..=1.0, progress.fraction().unwrap_or(0.0))
                    .length(140)
                    .girth(8),
            ]
            .spacing(GAP)
            .align_y(iced::Center)
            .into(),
            UpdateState::Ready(version) => row![
                small(format!("Updated to {version}")),
                btn("Restart")
                    .on_press(Message::Restart)
                    .style(button::primary),
            ]
            .spacing(GAP)
            .align_y(iced::Center)
            .into(),
            UpdateState::Failed(err) => row![
                single_line(err.as_str(), TEXT_SMALL).style(text::danger),
                btn("Retry")
                    .on_press(Message::CheckUpdates)
                    .style(style::action),
            ]
            .spacing(GAP)
            .align_y(iced::Center)
            .into(),
        }
    }
}
