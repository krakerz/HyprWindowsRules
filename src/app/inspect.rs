//! Window inspector: which rules hit which window.

use std::time::Duration;

use iced::widget::{Column, button, column, container, row, scrollable, text};
use iced::{Element, Fill, Font, Length, Task};
use serde_json::Value as Json;

use super::dialogs::{W_L, chrome, control_btn};
use super::style::{
    BOLD, GAP, LIST_W, MONO, PAD, ROW_H, SECTION_GAP, TEXT, TEXT_HEAD, TEXT_SMALL, U, section,
    selected_subtitle, single_line,
};
use super::{App, Geometry, Message, Modal};
use crate::inspect::{self, Finding, FindingKind};
use crate::model::{Rule, RuleKind};
use crate::{hypr, luaio};

/// Seconds to focus the window to pick before it is read.
const PICK_SECONDS: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickFor {
    #[default]
    Inspect,
    NewRule,
}

#[derive(Debug, Default)]
pub(super) struct State {
    pub findings: Vec<Finding>,
    pub counts: Vec<usize>,
    pub target: Option<usize>,
    pub countdown: Option<u8>,
    pub pick_for: PickFor,
    /// What `findings`/`counts` were computed from: the rules and `App::clients_gen`.
    analyzed: Option<(Vec<Rule>, u64)>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Open,
    Close,
    /// Pick an open window in the inspector's list.
    Select(usize),
    /// Jump from a finding to its rule and close the inspector.
    GoToRule(usize),
    PickActive(PickFor),
    Tick,
    ApplyNow(usize),
    Applied(Result<usize, String>),
}

fn tick() -> Task<Message> {
    Task::perform(tokio::time::sleep(Duration::from_secs(1)), |()| {
        Message::Inspect(Msg::Tick)
    })
}

impl App {
    pub(super) fn inspect_update(&mut self, msg: Msg) -> Task<Message> {
        match msg {
            Msg::Open => {
                self.refresh_windows();
                self.inspect.target = self.inspect.target.filter(|&i| i < self.clients.len());
                if self.inspect.target.is_none() && !self.clients.is_empty() {
                    self.inspect.target = Some(0);
                }
                self.modal = Some(Modal::Inspector);
            }
            Msg::Close => self.modal = None,
            Msg::Select(i) => self.inspect.target = Some(i),
            Msg::GoToRule(i) => {
                self.modal = None;
                self.list.kind = self.rules.get(i).map_or(self.list.kind, |r| r.kind);
                self.select(Some(i));
            }
            Msg::PickActive(pick_for) => {
                self.inspect.countdown = Some(PICK_SECONDS);
                self.inspect.pick_for = pick_for;
                self.modal = None;
                self.note = countdown_note(PICK_SECONDS);
                return tick();
            }
            Msg::Tick => return self.pick_tick(),
            Msg::ApplyNow(i) => return self.apply_now(i),
            Msg::Applied(Ok(n)) => {
                self.note = format!("Applied to {n} window{}", if n == 1 { "" } else { "s" });
            }
            Msg::Applied(Err(err)) => self.info("Apply now", err),
        }
        Task::none()
    }

    fn pick_tick(&mut self) -> Task<Message> {
        let Some(left) = self.inspect.countdown else {
            return Task::none();
        };
        if left > 1 {
            self.inspect.countdown = Some(left - 1);
            self.note = countdown_note(left - 1);
            return tick();
        }
        self.inspect.countdown = None;
        self.note.clear();
        let active = match hypr::active_window() {
            Ok(c) => c,
            Err(err) => {
                self.info("Pick window", err);
                return Task::none();
            }
        };
        self.refresh_windows();
        let Some(idx) = self
            .clients
            .iter()
            .position(|c| hypr::address(c) == hypr::address(&active))
        else {
            self.info("Pick window", "The focused window wasn't found.");
            return Task::none();
        };
        match self.inspect.pick_for {
            PickFor::Inspect => {
                self.inspect.target = Some(idx);
                self.modal = Some(Modal::Inspector);
            }
            PickFor::NewRule => {
                self.list.kind = RuleKind::Window;
                self.modal = Some(Modal::Picker {
                    clients: self.clients.clone(),
                    selected: Some(idx),
                    use_title: false,
                    copy_geom: true,
                    copy_ws: false,
                    for_current: None::<Geometry>,
                });
            }
        }
        Task::none()
    }

    fn apply_now(&mut self, i: usize) -> Task<Message> {
        let Some(rule) = self.rules.get(i) else {
            return Task::none();
        };
        if rule.kind != RuleKind::Window || !rule.enabled {
            self.info("Apply now", "Only enabled window rules can be applied.");
            return Task::none();
        }
        self.refresh_windows();
        let rule = &self.rules[i];
        let jobs: Vec<Vec<String>> = self
            .clients
            .iter()
            .filter(|c| rule.matches_window(c))
            .map(|c| inspect::apply_exprs(rule, c, &self.monitors))
            .collect();
        if jobs.is_empty() {
            self.info("Apply now", "No open window matches this rule.");
            return Task::none();
        }
        let result = jobs
            .iter()
            .try_for_each(|exprs| exprs.iter().try_for_each(|e| hypr::dispatch(e)))
            .map(|()| jobs.len());
        Task::done(Message::Inspect(Msg::Applied(result)))
    }

    pub(super) fn refresh_analysis(&mut self) {
        let fresh = self
            .inspect
            .analyzed
            .as_ref()
            .is_some_and(|(rules, gen_)| *gen_ == self.clients_gen && *rules == self.rules);
        if fresh {
            return;
        }
        self.inspect.counts = self
            .rules
            .iter()
            .map(|r| self.clients.iter().filter(|c| r.matches_window(c)).count())
            .collect();
        self.inspect.findings = inspect::analyze(&self.rules, &self.clients);
        self.inspect.analyzed = Some((self.rules.clone(), self.clients_gen));
    }

    pub(super) fn findings_for(&self, rule: usize) -> Vec<&Finding> {
        self.inspect
            .findings
            .iter()
            .filter(|f| f.rule == rule)
            .collect()
    }

    fn rule_title(&self, i: usize) -> String {
        self.rules.get(i).map_or_else(|| "?".into(), Rule::title)
    }

    pub(super) fn describe_finding(&self, f: &Finding) -> String {
        match &f.kind {
            FindingKind::NoOpenMatch => "matches no open window".into(),
            FindingKind::CatchAll => "matches every window".into(),
            FindingKind::Overridden { by, key } => {
                format!("“{key}” is overridden by “{}”", self.rule_title(*by))
            }
            FindingKind::Conflict { with, .. } => {
                format!(
                    "floats while “{}” tiles the same window",
                    self.rule_title(*with)
                )
            }
        }
    }

    pub(super) fn view_inspector(&self) -> Element<'_, Message> {
        let mut windows = Column::new().spacing(U / 2.0);
        for (i, c) in self.clients.iter().enumerate() {
            let selected = self.inspect.target == Some(i);
            let sub = single_line(hypr::str_field(c, "title"), TEXT_SMALL);
            let sub = if selected {
                sub.style(selected_subtitle)
            } else {
                sub.style(text::secondary)
            };
            let label = container(column![single_line(hypr::str_field(c, "class"), TEXT), sub])
                .width(Fill)
                .center_y(Fill)
                .clip(true);
            windows = windows.push(
                button(label)
                    .width(Fill)
                    .height(ROW_H)
                    .padding([0.0, PAD])
                    .on_press(Message::Inspect(Msg::Select(i)))
                    .style(if selected {
                        button::primary
                    } else {
                        button::text
                    }),
            );
        }
        let pick_label = match self.inspect.countdown {
            Some(n) => format!("Focus the window… {n}"),
            None => format!("Pick active window ({PICK_SECONDS} s)"),
        };
        let pick = control_btn(pick_label).width(Fill).on_press_maybe(
            self.inspect
                .countdown
                .is_none()
                .then_some(Message::Inspect(Msg::PickActive(PickFor::Inspect))),
        );
        let left = column![
            text("Open windows").size(TEXT_HEAD).font(BOLD),
            scrollable(windows).width(Fill).height(Fill),
            pick,
        ]
        .spacing(GAP)
        .width(LIST_W);

        let right = match self.inspect.target.and_then(|i| self.clients.get(i)) {
            Some(c) => self.inspector_detail(c),
            None => text("No open windows.").size(TEXT).into(),
        };
        let right = column![right, self.rule_checks()].spacing(SECTION_GAP);

        let body = row![left, scrollable(right).width(Fill).height(Fill)]
            .spacing(SECTION_GAP)
            .height(BODY_H);
        chrome(
            "Inspector",
            W_L,
            body.into(),
            vec![
                control_btn("Close")
                    .on_press(Message::Inspect(Msg::Close))
                    .style(button::primary)
                    .into(),
            ],
        )
    }

    fn inspector_detail<'a>(&'a self, client: &'a Json) -> Element<'a, Message> {
        let mut matching = column![].spacing(GAP);
        let mut any = false;
        for (i, rule) in self.rules.iter().enumerate() {
            if !(rule.enabled && rule.matches_window(client)) {
                continue;
            }
            any = true;
            let overridden: Vec<String> = self
                .findings_for(i)
                .into_iter()
                .filter_map(|f| match &f.kind {
                    FindingKind::Overridden { key, .. } => Some(key.clone()),
                    _ => None,
                })
                .collect();
            let mut line = format!("{}. {}: {}", i + 1, rule.title(), rule.summary());
            if !overridden.is_empty() {
                line.push_str(&format!("  (overridden: {})", overridden.join(", ")));
            }
            matching = matching.push(text(line).size(TEXT));
        }
        if !any {
            matching = matching.push(hint("No rule matches this window."));
        }

        let mut result_col = column![].spacing(GAP);
        let result = inspect::effective(&self.rules, client);
        if result.is_empty() {
            result_col = result_col.push(hint("No effects."));
        }
        // key | value | ← rule, aligned across rows by fixed column portions
        for (key, value, idx) in result {
            result_col = result_col.push(
                row![
                    cell(key, MONO, 4),
                    cell(luaio::lua_value(&value, 0), Font::DEFAULT, 3),
                    cell(format!("← {}", self.rule_title(idx)), Font::DEFAULT, 3),
                ]
                .spacing(GAP),
            );
        }
        column![
            section("Matching rules", matching),
            section("Final result", result_col),
        ]
        .spacing(SECTION_GAP)
        .into()
    }

    fn rule_checks(&self) -> Element<'_, Message> {
        let mut list = Column::new().spacing(U / 2.0);
        if self.inspect.findings.is_empty() {
            list = list.push(hint("Nothing to report."));
        }
        for f in &self.inspect.findings {
            let label = format!("{}: {}", self.rule_title(f.rule), self.describe_finding(f));
            list = list.push(
                button(container(single_line(label, TEXT)).clip(true))
                    .width(Fill)
                    .padding(0)
                    .on_press(Message::Inspect(Msg::GoToRule(f.rule)))
                    .style(button::text),
            );
        }
        section("Rule checks", list)
    }
}

/// Height of the inspector's two columns.
const BODY_H: f32 = 420.0;

fn hint(s: &str) -> Element<'_, Message> {
    text(s)
        .size(TEXT_SMALL)
        .font(super::style::ITALIC)
        .style(text::secondary)
        .into()
}

/// One clipped, single-line cell of the result grid.
fn cell<'a>(s: impl text::IntoFragment<'a>, font: Font, portion: u16) -> Element<'a, Message> {
    container(single_line(s, TEXT).font(font))
        .width(Length::FillPortion(portion))
        .clip(true)
        .into()
}

fn countdown_note(n: u8) -> String {
    format!("Focus the window to pick… {n}")
}
