//! Rule list.

use iced::widget::{
    Column, button, column, container, mouse_area, opaque, row, scrollable, space, stack, text,
    text_input, toggler, tooltip,
};
use iced::{Element, Fill, Font, Point, Task, mouse};

use super::inspect::{self, PickFor};
use super::style::{
    self, BOLD, CONTROL_H, GAP, ITALIC, LIST_W, PAD, RADIUS, ROW_H, TEXT, TEXT_SMALL, U,
};
use super::{App, Message};
use crate::inspect::{Finding, FindingKind};
use crate::model::{Rule, RuleKind};

/// Widget id of the filter box (Ctrl+F focuses it).
pub(super) const FILTER_ID: &str = "rule-filter";

/// Width of the drag handle column in a row.
const HANDLE_W: f32 = 16.0;
/// Width of the badge column (count badge, warning badge) and of the group toggler column.
const BADGE_W: f32 = 64.0;
/// Width of the context menu.
const MENU_W: f32 = 200.0;

#[derive(Debug, Default)]
pub(super) struct State {
    pub kind: RuleKind,
    pub collapsed: Vec<String>,
    pub drag: Option<usize>,
    pub drop_at: Option<usize>,
    /// Last mouse position inside the list area (relative to it).
    pub cursor: Point,
    /// Open context menu: rule index and where it was opened.
    pub menu: Option<(usize, Point)>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Kind(RuleKind),
    Collapse(String),
    ToggleGroup(String, bool),
    DragStart(usize),
    DragOver(usize),
    DragEnd,
    DragCancel,
    Cursor(Point),
    Menu(usize),
    CloseMenu,
    Ungroup(usize),
    /// Context-menu actions: close the menu, then act on rule `i`.
    Duplicate(usize),
    Delete(usize),
    Apply(usize),
}

/// Moves the element at `from` so it ends up at index `to`.
fn move_to<T>(items: &mut Vec<T>, from: usize, to: usize) {
    let item = items.remove(from);
    items.insert(to, item);
}

impl App {
    pub(super) fn list_update(&mut self, msg: Msg) -> Task<Message> {
        match msg {
            Msg::Kind(k) => {
                self.list.kind = k;
                self.list.menu = None;
                let first = self.rules.iter().position(|r| r.kind == k);
                self.select(first);
            }
            Msg::Collapse(name) => {
                if let Some(pos) = self.list.collapsed.iter().position(|n| *n == name) {
                    self.list.collapsed.remove(pos);
                } else {
                    self.list.collapsed.push(name);
                }
            }
            Msg::ToggleGroup(name, on) => {
                if let Some(g) = self.groups.iter_mut().find(|g| g.name == name) {
                    g.enabled = on;
                    self.dirty = true;
                }
            }
            Msg::DragStart(i) => {
                self.list.drag = Some(i);
                self.list.drop_at = Some(i);
            }
            Msg::DragOver(i) => {
                if self.list.drag.is_some() {
                    self.list.drop_at = Some(i);
                }
            }
            Msg::DragEnd => self.finish_drag(),
            Msg::DragCancel => {
                self.list.drag = None;
                self.list.drop_at = None;
            }
            Msg::Cursor(p) => {
                if self.list.menu.is_none() {
                    self.list.cursor = p;
                }
            }
            Msg::Menu(i) => {
                if i < self.rules.len() {
                    self.select(Some(i));
                    self.list.menu = Some((i, self.list.cursor));
                }
            }
            Msg::CloseMenu => self.list.menu = None,
            Msg::Ungroup(i) => self.ungroup(i),
            Msg::Duplicate(i) => {
                self.list.menu = None;
                self.select(Some(i));
                return self.handle(Message::Duplicate);
            }
            Msg::Delete(i) => {
                self.list.menu = None;
                self.select(Some(i));
                return self.handle(Message::Delete);
            }
            Msg::Apply(i) => {
                self.list.menu = None;
                return self.handle(Message::Inspect(inspect::Msg::ApplyNow(i)));
            }
        }
        Task::none()
    }

    /// Drop the dragged rule on the hovered row: it takes that row's position and joins
    /// the group around it.
    fn finish_drag(&mut self) {
        let (from, to) = (self.list.drag.take(), self.list.drop_at.take());
        let (Some(from), Some(to)) = (from, to) else {
            return;
        };
        if from == to || from >= self.rules.len() || to >= self.rules.len() {
            return;
        }
        move_to(&mut self.rules, from, to);
        let above = to.checked_sub(1).map(|j| &self.rules[j].group);
        let below = self.rules.get(to + 1).map(|r| &r.group);
        let group = match (above, below) {
            (Some(a), Some(b)) if a == b => Some(a.clone()),
            (_, Some(b)) => Some(b.clone()),
            _ => None,
        };
        if let Some(group) = group {
            self.rules[to].group = group;
        }
        self.ensure_groups();
        self.dirty = true;
        self.select(Some(to));
    }

    /// Takes rule `i` out of its group, placing it right after the group's block.
    fn ungroup(&mut self, i: usize) {
        let Some(group) = self.rules.get(i).map(|r| r.group.clone()) else {
            return;
        };
        if group.is_empty() {
            return;
        }
        let mut end = i;
        while self.rules.get(end + 1).is_some_and(|r| r.group == group) {
            end += 1;
        }
        self.rules[i].group.clear();
        move_to(&mut self.rules, i, end);
        self.ensure_groups();
        self.dirty = true;
        self.list.menu = None;
        self.select(Some(end));
    }

    pub(super) fn view_list(&self) -> Element<'_, Message> {
        let kind = self.list.kind;
        let window = kind == RuleKind::Window;
        let last = RuleKind::ALL.len() - 1;
        let kinds = RuleKind::ALL
            .iter()
            .enumerate()
            .fold(row![].spacing(0), |r, (n, &k)| {
                let active = k == kind;
                // Joined segments: only the outer corners are rounded.
                let radius = match n {
                    0 => iced::border::left(RADIUS),
                    n if n == last => iced::border::right(RADIUS),
                    _ => iced::border::Radius::default(),
                };
                r.push(
                    button(centered(k.label(), TEXT))
                        .width(Fill)
                        .height(CONTROL_H)
                        .padding([0.0, GAP])
                        .on_press(Message::List(Msg::Kind(k)))
                        .style(move |theme, status| {
                            let mut s = if active {
                                button::primary(theme, status)
                            } else {
                                style::action(theme, status)
                            };
                            s.border.radius = radius;
                            s
                        }),
                )
            });
        let tool = |label: &'static str, tip: &'static str, msg: Message| {
            with_tip(
                button(centered(label, TEXT))
                    .width(Fill)
                    .height(CONTROL_H)
                    .padding([0.0, U])
                    .on_press(msg)
                    .style(style::action),
                tip.to_string(),
            )
        };
        let mut tools = row![tool("+ New", "Add an empty rule", Message::AddRule)].spacing(GAP);
        if window {
            tools = tools
                .push(tool(
                    "From window",
                    "Create a rule from an open window",
                    Message::FromWindow,
                ))
                .push(tool(
                    "Pick (3 s)",
                    "Focus a window within 3 seconds to create a rule from it",
                    Message::Inspect(inspect::Msg::PickActive(PickFor::NewRule)),
                ));
        }

        let needle = self.search.to_lowercase();
        let filtering = !needle.is_empty();
        let mut items = Column::new().spacing(2);
        let mut last_group = "";
        let mut shown = 0;
        for (i, r) in self.rules.iter().enumerate() {
            if r.kind != kind {
                continue;
            }
            let (title, summary) = (r.title(), r.summary());
            if filtering
                && !format!("{title}\n{summary}")
                    .to_lowercase()
                    .contains(&needle)
            {
                continue;
            }
            if r.group != last_group {
                last_group = &r.group;
                if !r.group.is_empty() {
                    items = items.push(self.group_header(&r.group, kind));
                }
            }
            if !filtering && self.list.collapsed.contains(&r.group) {
                continue;
            }
            shown += 1;
            let dragging = self.list.drag.is_some();
            let marker = || {
                container(space())
                    .width(Fill)
                    .height(2)
                    .style(style::rule_line)
            };
            let drop_here = dragging && self.list.drop_at == Some(i);
            let below = self.list.drag.is_some_and(|from| i > from);
            if drop_here && !below {
                items = items.push(marker());
            }
            let entry = mouse_area(self.view_row(i, r, &title, &summary))
                .on_enter(Message::List(Msg::DragOver(i)))
                .on_right_press(Message::List(Msg::Menu(i)));
            items = items.push(entry);
            if drop_here && below {
                items = items.push(marker());
            }
        }
        if shown == 0 {
            let what = if filtering {
                "No rules match the filter".to_string()
            } else {
                format!("No {} rules yet", kind.label().to_lowercase())
            };
            items = items.push(
                text(what)
                    .size(TEXT_SMALL)
                    .font(ITALIC)
                    .style(text::secondary),
            );
        }
        let scroll = mouse_area(
            scrollable(items.padding(iced::Padding::ZERO.right(GAP)))
                .width(Fill)
                .height(Fill),
        )
        .on_release(Message::List(Msg::DragEnd))
        .on_exit(Message::List(Msg::DragCancel))
        .on_move(|p| Message::List(Msg::Cursor(p)));
        let list: Element<'_, Message> = match self.list.menu {
            Some((i, at)) => stack![scroll, self.context_menu(i, at)].into(),
            None => scroll.into(),
        };
        container(
            column![
                kinds,
                tools,
                text_input("Filter rules…", &self.search)
                    .id(FILTER_ID)
                    .size(TEXT)
                    .padding([0.0, 12.0])
                    .line_height(text::LineHeight::Absolute(CONTROL_H.into()))
                    .on_input(Message::Search),
                list,
            ]
            .spacing(GAP),
        )
        .width(LIST_W)
        .padding(PAD)
        .into()
    }

    /// Right-click menu: a click-catcher over the list with the menu placed at `at`.
    fn context_menu(&self, i: usize, at: Point) -> Element<'_, Message> {
        let Some(rule) = self.rules.get(i) else {
            return space().into();
        };
        let item = |label: &'static str, msg: Msg| {
            button(text(label).size(TEXT).height(Fill).align_y(iced::Center))
                .width(Fill)
                .height(CONTROL_H)
                .padding([0.0, 12.0])
                .on_press(Message::List(msg))
                .style(button::text)
        };
        let mut menu = column![
            item("Duplicate", Msg::Duplicate(i)),
            item("Delete", Msg::Delete(i)),
        ];
        if rule.kind == RuleKind::Window {
            menu = menu.push(item("Apply now", Msg::Apply(i)));
        }
        if !rule.group.is_empty() {
            menu = menu.push(item("Remove from group", Msg::Ungroup(i)));
        }
        // Keep the menu inside the list column
        let max_x = (LIST_W - 2.0 * PAD - MENU_W).max(0.0);
        let x = at.x.clamp(0.0, max_x);
        let menu = container(menu)
            .width(MENU_W)
            .padding(U)
            .style(container::bordered_box);
        let positioned = container(menu).padding(iced::Padding {
            top: at.y.max(0.0),
            left: x,
            right: 0.0,
            bottom: 0.0,
        });
        opaque(
            mouse_area(positioned.width(Fill).height(Fill))
                .on_press(Message::List(Msg::CloseMenu))
                .on_right_press(Message::List(Msg::CloseMenu)),
        )
    }

    fn group_header<'a>(&'a self, name: &'a str, kind: RuleKind) -> Element<'a, Message> {
        let count = self
            .rules
            .iter()
            .filter(|r| r.kind == kind && r.group == name)
            .count();
        let collapsed = self.list.collapsed.iter().any(|n| n == name);
        let enabled = self
            .groups
            .iter()
            .find(|g| g.name == name)
            .is_none_or(|g| g.enabled);
        let arrow = if collapsed { "▸" } else { "▾" };
        let owned = name.to_string();
        let head = row![
            text(arrow).size(TEXT).width(HANDLE_W),
            container(style::single_line(name, TEXT).font(BOLD))
                .width(Fill)
                .clip(true),
            style::pill_badge(count.to_string(), style::badge),
        ]
        .spacing(GAP)
        .height(Fill)
        .align_y(iced::Center);
        row![
            button(head)
                .width(Fill)
                .height(CONTROL_H)
                .padding([0.0, GAP])
                .on_press(Message::List(Msg::Collapse(name.to_string())))
                .style(button::text),
            container(
                row![
                    text(if enabled { "on" } else { "off" })
                        .size(TEXT_SMALL)
                        .style(text::secondary),
                    toggler(enabled)
                        .size(16)
                        .on_toggle(move |b| Message::List(Msg::ToggleGroup(owned.clone(), b))),
                ]
                .spacing(U)
                .align_y(iced::Center)
            )
            .width(BADGE_W)
            .align_x(iced::alignment::Horizontal::Right),
        ]
        .spacing(GAP)
        .height(CONTROL_H)
        .align_y(iced::Center)
        .into()
    }

    fn view_row<'a>(
        &'a self,
        i: usize,
        r: &'a Rule,
        title: &str,
        summary: &str,
    ) -> Element<'a, Message> {
        let selected = self.selected == Some(i);
        let off = if r.enabled { "" } else { "   (off)" };
        let modified = if self.is_rule_modified(i) { " •" } else { "" };
        let subtitle = style::single_line(summary.to_string(), TEXT_SMALL);
        let subtitle = if selected {
            subtitle.style(style::selected_subtitle)
        } else {
            subtitle.style(text::secondary)
        };
        let label =
            container(
                column![
                    style::single_line(format!("{title}{modified}{off}"), TEXT)
                        .font(if r.enabled { Font::DEFAULT } else { ITALIC }),
                    subtitle,
                ]
                .spacing(2),
            )
            .width(Fill)
            .clip(true);

        let mut badges = row![].spacing(U).align_y(iced::Center);
        let matches = self.inspect.counts.get(i).copied().unwrap_or(0);
        if matches > 0 {
            badges = badges.push(with_tip(
                style::pill_badge(matches.to_string(), style::badge),
                format!("Matches {matches} open window(s)"),
            ));
        }
        // Apps that aren't running are normal, so "no open match" stays out of the list badge
        let findings: Vec<_> = self
            .findings_for(i)
            .into_iter()
            .filter(|f| f.kind != FindingKind::NoOpenMatch)
            .collect();
        if !findings.is_empty() {
            let tip = findings
                .iter()
                .map(|f| self.finding_tip(f))
                .collect::<Vec<_>>()
                .join("\n");
            badges = badges.push(with_tip(style::pill_badge("⚠", style::warn_badge), tip));
        }
        let badges = container(badges)
            .width(BADGE_W)
            .align_x(iced::alignment::Horizontal::Right);

        let handle_text = text("⠿").size(TEXT).width(HANDLE_W);
        let handle_text = if selected {
            handle_text.style(style::selected_subtitle)
        } else {
            handle_text.style(text::secondary)
        };
        let handle = mouse_area(handle_text)
            .on_press(Message::List(Msg::DragStart(i)))
            .interaction(mouse::Interaction::Grab);
        // Buttons top-align their content, so the row fills the height to centre vertically
        let content = row![handle, label, badges]
            .spacing(GAP)
            .height(Fill)
            .align_y(iced::Center);
        let style = if selected {
            button::primary
        } else {
            button::text
        };
        button(content)
            .width(Fill)
            .height(ROW_H)
            .padding([0.0, GAP])
            .on_press(Message::Select(i))
            .style(style)
            .into()
    }

    fn finding_tip(&self, f: &Finding) -> String {
        let title = |i: usize| self.rules.get(i).map_or_else(String::new, Rule::title);
        match &f.kind {
            FindingKind::NoOpenMatch => "Matches no open window".into(),
            FindingKind::CatchAll => "Matches every window".into(),
            FindingKind::Overridden { by, key } => {
                format!("“{key}” is overridden by “{}”", title(*by))
            }
            FindingKind::Conflict { with, key } => {
                format!("“{key}” conflicts with “{}”", title(*with))
            }
        }
    }
}

/// Button label centred in both directions.
fn centered<'a>(label: &'a str, size: f32) -> iced::widget::Text<'a> {
    style::single_line(label, size)
        .width(Fill)
        .height(Fill)
        .align_x(iced::Center)
        .align_y(iced::Center)
}

fn with_tip<'a>(content: impl Into<Element<'a, Message>>, tip: String) -> Element<'a, Message> {
    tooltip(
        content,
        container(text(tip).size(TEXT_SMALL))
            .padding(GAP)
            .style(container::rounded_box),
        tooltip::Position::Bottom,
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_first_to_last_and_back() {
        let mut v = vec![1, 2, 3, 4];
        move_to(&mut v, 0, 3);
        assert_eq!(v, [2, 3, 4, 1]);
        move_to(&mut v, 3, 0);
        assert_eq!(v, [1, 2, 3, 4]);
    }

    #[test]
    fn move_to_same_position_is_noop() {
        let mut v = vec![1, 2, 3];
        move_to(&mut v, 1, 1);
        assert_eq!(v, [1, 2, 3]);
    }
}
