//! Saved-version history with diff and restore.

use iced::widget::{Column, button, column, container, row, scrollable, text};
use iced::{Element, Fill, Task, Theme};

use super::dialogs::{W_L, W_S, chrome, control_btn};
use super::style::{CONTROL_H, GAP, MONO, PAD, SECTION_GAP, TEXT, TEXT_SMALL, U, single_line};
use super::{App, Message, Modal};
use crate::diff::{self, Line};
use crate::luaio;

/// Unchanged lines kept around each change in the diff view.
const CONTEXT: usize = 3;

#[derive(Debug, Default)]
pub(super) struct State {
    pub entries: Vec<luaio::Backup>,
    pub selected: Option<usize>,
    pub diff: Vec<diff::Line>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Open,
    Select(usize),
    Restore,
    RestoreConfirmed,
}

/// A diff line, or a run of unchanged lines folded away.
#[derive(Debug, PartialEq)]
enum Row<'a> {
    Line(&'a Line),
    Folded(usize),
}

/// Keep changed lines and `CONTEXT` lines around them; fold the rest.
fn fold(diff: &[Line]) -> Vec<Row<'_>> {
    let changed: Vec<usize> = diff
        .iter()
        .enumerate()
        .filter(|(_, l)| !matches!(l, Line::Same(_)))
        .map(|(i, _)| i)
        .collect();
    let keep = |i: usize| changed.iter().any(|&c| c.abs_diff(i) <= CONTEXT);
    let mut rows = Vec::new();
    let mut folded = 0;
    for (i, line) in diff.iter().enumerate() {
        if keep(i) {
            if folded > 0 {
                rows.push(Row::Folded(folded));
                folded = 0;
            }
            rows.push(Row::Line(line));
        } else {
            folded += 1;
        }
    }
    if folded > 0 {
        rows.push(Row::Folded(folded));
    }
    rows
}

impl App {
    pub(super) fn history_update(&mut self, msg: Msg) -> Task<Message> {
        match msg {
            Msg::Open => {
                self.history.entries = luaio::list_backups(&self.settings.rules());
                self.select_backup(if self.history.entries.is_empty() {
                    None
                } else {
                    Some(0)
                });
                self.modal = Some(Modal::History);
            }
            Msg::Select(i) => self.select_backup(Some(i)),
            Msg::Restore => {
                let Some(entry) = self
                    .history
                    .selected
                    .and_then(|i| self.history.entries.get(i))
                else {
                    return Task::none();
                };
                let mut body = format!(
                    "Restore the rules from {}? Current rules are backed up first.",
                    entry.stamp
                );
                if self.dirty {
                    body.push_str("\n\nUnsaved changes will be lost.");
                }
                self.modal = Some(Modal::Message {
                    title: "Restore this version".into(),
                    body,
                    buttons: vec![
                        ("Restore".into(), Message::History(Msg::RestoreConfirmed)),
                        ("Cancel".into(), Message::CloseModal),
                    ],
                });
            }
            Msg::RestoreConfirmed => {
                let Some(entry) = self
                    .history
                    .selected
                    .and_then(|i| self.history.entries.get(i))
                    .cloned()
                else {
                    return Task::none();
                };
                self.modal = None;
                match luaio::restore_backup(&self.settings.rules(), &entry.path) {
                    Ok(_) => {
                        self.load_rules();
                        self.note = format!("Restored {}", entry.stamp);
                    }
                    Err(err) => self.info("Restore failed", err.to_string()),
                }
            }
        }
        Task::none()
    }

    /// Select a backup and diff it (old) against the rules file as it is now (new).
    fn select_backup(&mut self, idx: Option<usize>) {
        self.history.selected = idx.filter(|&i| i < self.history.entries.len());
        self.history.diff = match self.history.selected.map(|i| &self.history.entries[i]) {
            Some(entry) => {
                let old = std::fs::read_to_string(&entry.path).unwrap_or_default();
                let new = std::fs::read_to_string(self.settings.rules()).unwrap_or_default();
                diff::lines(&old, &new)
            }
            None => Vec::new(),
        };
    }

    pub(super) fn view_history(&self) -> Element<'_, Message> {
        let h = &self.history;
        let close = control_btn("Close").on_press(Message::CloseModal);
        if h.entries.is_empty() {
            return chrome(
                "History",
                W_S,
                text("No backups yet (one is made on every save).")
                    .size(TEXT)
                    .into(),
                vec![close.style(button::primary).into()],
            );
        }
        let mut stamps = Column::new().spacing(U / 2.0);
        for (i, e) in h.entries.iter().enumerate() {
            let style = if h.selected == Some(i) {
                button::primary
            } else {
                button::text
            };
            stamps = stamps.push(
                button(container(single_line(e.stamp.as_str(), TEXT)).center_y(Fill))
                    .width(Fill)
                    .height(CONTROL_H)
                    .padding([0.0, PAD])
                    .on_press(Message::History(Msg::Select(i)))
                    .style(style),
            );
        }
        let mut lines = Column::new();
        if h.diff.iter().all(|l| matches!(l, Line::Same(_))) {
            lines = lines.push(text("Same as the current file.").size(TEXT_SMALL));
        } else {
            for row in fold(&h.diff) {
                // Diff is old → new: Removed lines are what the backup has and the file lost
                lines = lines.push(match row {
                    Row::Folded(n) => text(format!("… {n} unchanged lines"))
                        .size(TEXT_SMALL)
                        .font(super::style::ITALIC)
                        .style(text::secondary)
                        .into(),
                    Row::Line(Line::Same(s)) => diff_row("  ", s, None),
                    Row::Line(Line::Added(s)) => diff_row("+ ", s, Some(DiffKind::Added)),
                    Row::Line(Line::Removed(s)) => diff_row("- ", s, Some(DiffKind::Removed)),
                });
            }
        }
        let body = column![
            text("Differences from the selected version to the current file.")
                .size(TEXT_SMALL)
                .style(text::secondary),
            row![
                scrollable(stamps).width(STAMP_W).height(Fill),
                scrollable(lines).width(Fill).height(Fill),
            ]
            .spacing(SECTION_GAP)
            .height(BODY_H),
        ]
        .spacing(GAP);
        chrome(
            "History",
            W_L,
            body.into(),
            vec![
                close.into(),
                control_btn("Restore")
                    .on_press_maybe(h.selected.map(|_| Message::History(Msg::Restore)))
                    .style(button::primary)
                    .into(),
            ],
        )
    }
}

/// Width of the stamp list and height of the list/diff row.
const STAMP_W: f32 = 200.0;
const BODY_H: f32 = 420.0;
/// Tab stops render as tofu boxes in the diff font, so tabs become spaces.
const TAB: &str = "    ";
/// Opacity of the added/removed row backgrounds.
const TINT: f32 = 0.15;

#[derive(Clone, Copy)]
enum DiffKind {
    Added,
    Removed,
}

fn diff_row<'a>(marker: &str, s: &str, kind: Option<DiffKind>) -> Element<'a, Message> {
    let gutter = text(marker.to_string()).size(TEXT_SMALL).font(MONO);
    let gutter = match kind {
        Some(DiffKind::Added) => gutter.style(text::success),
        Some(DiffKind::Removed) => gutter.style(text::danger),
        None => gutter,
    };
    let line = row![
        gutter,
        text(s.replace('\t', TAB)).size(TEXT_SMALL).font(MONO)
    ];
    container(line)
        .width(Fill)
        .padding([0.0, U])
        .style(move |theme: &Theme| {
            let p = theme.extended_palette();
            let tint = match kind {
                Some(DiffKind::Added) => Some(p.success.base.color),
                Some(DiffKind::Removed) => Some(p.danger.base.color),
                None => None,
            };
            container::Style {
                background: tint.map(|c| c.scale_alpha(TINT).into()),
                ..Default::default()
            }
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(s: &str) -> Line {
        Line::Same(s.into())
    }

    #[test]
    fn folds_unchanged_runs_beyond_the_context() {
        let mut diff: Vec<Line> = (0..10).map(|i| same(&i.to_string())).collect();
        diff.insert(5, Line::Added("x".into()));
        let rows = fold(&diff);
        // 0,1 folded; 2..=4 context; the change; 5..=7 context; 8,9 folded
        assert_eq!(rows.first(), Some(&Row::Folded(2)));
        assert_eq!(rows.last(), Some(&Row::Folded(2)));
        assert_eq!(rows.len(), 1 + 3 + 1 + 3 + 1);
    }

    #[test]
    fn tabs_become_spaces_in_diff_rows() {
        assert_eq!("\ta\tb".replace('\t', TAB), "    a    b");
    }

    #[test]
    fn a_diff_without_changes_folds_to_one_row() {
        assert_eq!(fold(&[same("a"), same("b")]), [Row::Folded(2)]);
        assert!(fold(&[]).is_empty());
    }
}
