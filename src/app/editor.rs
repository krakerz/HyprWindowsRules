//! Rule editor form.

use std::borrow::Borrow;

use iced::widget::{
    Button, Column, PickList, Text, TextInput, button, center, checkbox, column, combo_box,
    container, pick_list, responsive, row, scrollable, space, text, text_input, toggler, tooltip,
};
use iced::{Element, Fill, Length, Pixels};
use serde_json::Value as Json;

use super::style::{
    self, CONTROL_H, FORM_MAX_W, GAP, LABEL_W, MONO, PAD, SECTION_GAP, SIDE_W, TEXT, TEXT_SMALL,
    form_row, section, single_line,
};
use super::{App, COMMON_KEYS, Geometry, Message, inspect};
use crate::inspect::FindingKind;
use crate::model::{self, Kind, Mode, PATTERN_KEYS, Rule, RuleKind, Value};
use crate::{hypr, luaio};

// ---- value <-> text for the free-form property table ------------------------------------

pub(super) fn parse_value(text: &str) -> Value {
    let t = text.trim();
    if t == "true" || t == "false" {
        return Value::Bool(t == "true");
    }
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let unsigned = t.strip_prefix('-').unwrap_or(t);
    if digits(unsigned)
        && let Ok(i) = t.parse()
    {
        return Value::Int(i);
    }
    if let Some((a, b)) = unsigned.split_once('.')
        && (a.is_empty() || digits(a))
        && digits(b)
        && let Ok(f) = t.parse()
    {
        return Value::Float(f);
    }
    if t.len() >= 2 && t.starts_with('{') && t.ends_with('}') {
        let inner = t[1..t.len() - 1].trim();
        return Value::List(if inner.is_empty() {
            vec![]
        } else {
            inner.split(',').map(parse_value).collect()
        });
    }
    if t.len() >= 2
        && (t.starts_with('"') && t.ends_with('"') || t.starts_with('\'') && t.ends_with('\''))
    {
        return Value::Str(t[1..t.len() - 1].into());
    }
    Value::Str(t.into())
}

pub(super) fn format_value(v: &Value) -> String {
    match v {
        Value::List(items) => format!(
            "{{ {} }}",
            items
                .iter()
                .map(format_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Map(_) => luaio::lua_value(v, 0),
        // Keep strings like "100" strings on the round trip
        Value::Str(s)
            if s == "true"
                || s == "false"
                || (!s.is_empty()
                    && s.chars()
                        .all(|c| c.is_ascii_digit() || c == '.' || c == '-')) =>
        {
            format!("\"{s}\"")
        }
        v => v.plain(),
    }
}

// ---- small enums for pick lists ---------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyOpt(pub(super) &'static str);

impl std::fmt::Display for KeyOpt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(model::match_key_label(self.0))
    }
}

pub(super) const KEY_OPTS: [KeyOpt; 7] = [
    KeyOpt(PATTERN_KEYS[0]),
    KeyOpt(PATTERN_KEYS[1]),
    KeyOpt(PATTERN_KEYS[2]),
    KeyOpt(PATTERN_KEYS[3]),
    KeyOpt(PATTERN_KEYS[4]),
    KeyOpt(PATTERN_KEYS[5]),
    KeyOpt(PATTERN_KEYS[6]),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatState {
    Leave,
    Float,
    NoFloat,
    Tile,
}

impl FloatState {
    const ALL: [FloatState; 4] = [
        FloatState::Leave,
        FloatState::Float,
        FloatState::NoFloat,
        FloatState::Tile,
    ];
}

impl std::fmt::Display for FloatState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            FloatState::Leave => "Leave as is",
            FloatState::Float => "Floating",
            FloatState::NoFloat => "Not floating (float = false)",
            FloatState::Tile => "Tiled",
        })
    }
}

// ---- state ------------------------------------------------------------------------------

/// Text-field state of the rule editor; written back into the rule's effects on every edit.
#[derive(Debug, Clone)]
pub(super) struct Editor {
    pub(super) kind: RuleKind,
    pub(super) name: String,
    pub(super) group: String,
    pub(super) enabled: bool,
    pub(super) state: FloatState,
    pub(super) size_w: String,
    pub(super) size_h: String,
    pub(super) move_x: String,
    pub(super) move_y: String,
    pub(super) center: bool,
    pub(super) workspace: String,
    pub(super) ws_silent: bool,
    pub(super) monitor: String,
    pub(super) pin: bool,
    pub(super) no_focus: bool,
    pub(super) props: Vec<(String, String)>,
    /// The values behind `props` as loaded, reused while their text is unchanged: the text
    /// form doesn't round-trip every value (tables, 1.0 vs 1, strings with commas).
    pub(super) loaded_props: Vec<(String, String, Value)>,
}

/// Whether the Common widgets show `value` for `key` without losing anything; effects
/// that don't fit go to "Other properties" instead of being dropped on the next edit.
pub(super) fn fits_common(rule: &Rule, key: &str, value: &Value) -> bool {
    let scalar = |v: &Value| matches!(v, Value::Str(_) | Value::Int(_));
    match key {
        "float" => matches!(value, Value::Bool(_)),
        "tile" => *value == Value::Bool(true) && rule.effect("float").is_none(),
        "center" | "pin" | "no_initial_focus" => *value == Value::Bool(true),
        "size" | "move" => matches!(value, Value::List(v) if v.len() == 2 && v.iter().all(scalar)),
        "workspace" | "monitor" => scalar(value),
        _ => false,
    }
}

impl Editor {
    pub(super) fn from_rule(rule: &Rule) -> Self {
        let window = rule.kind == RuleKind::Window;
        let common = |key: &str| {
            rule.effect(key)
                .filter(|v| window && fits_common(rule, key, v))
        };
        let pair = |key: &str| match common(key) {
            Some(Value::List(v)) => (v[0].plain(), v[1].plain()),
            _ => Default::default(),
        };
        let (size_w, size_h) = pair("size");
        let (move_x, move_y) = pair("move");
        let state = match (common("float"), common("tile")) {
            (Some(Value::Bool(true)), _) => FloatState::Float,
            (Some(Value::Bool(false)), _) => FloatState::NoFloat,
            (_, Some(_)) => FloatState::Tile,
            _ => FloatState::Leave,
        };
        let flag = |key: &str| common(key).is_some();
        let ws = match rule.kind {
            RuleKind::Window => common("workspace"),
            RuleKind::Workspace => rule.effect("workspace"),
            RuleKind::Layer => None,
        }
        .map(Value::plain)
        .unwrap_or_default();
        let loaded_props: Vec<(String, String, Value)> = rule
            .effects
            .iter()
            .filter(|(k, v)| match rule.kind {
                RuleKind::Window => !COMMON_KEYS.contains(&k.as_str()) || !fits_common(rule, k, v),
                RuleKind::Workspace => k != "workspace",
                RuleKind::Layer => true,
            })
            .map(|(k, v)| (k.clone(), format_value(v), v.clone()))
            .collect();
        let ws_silent = window && ws.ends_with(" silent");
        Self {
            kind: rule.kind,
            name: rule.name.clone(),
            group: rule.group.clone(),
            enabled: rule.enabled,
            state,
            size_w,
            size_h,
            move_x,
            move_y,
            center: flag("center"),
            workspace: if window {
                ws.strip_suffix(" silent").unwrap_or(&ws).to_string()
            } else {
                ws
            },
            ws_silent,
            monitor: common("monitor").map(Value::plain).unwrap_or_default(),
            pin: flag("pin"),
            no_focus: flag("no_initial_focus"),
            props: loaded_props
                .iter()
                .map(|(k, text, _)| (k.clone(), text.clone()))
                .collect(),
            loaded_props,
        }
    }

    pub(super) fn effects(&self) -> Vec<(String, Value)> {
        let mut e: Vec<(String, Value)> = Vec::new();
        let s = |v: &str| Value::Str(v.trim().to_string());
        match self.kind {
            RuleKind::Window => self.common_effects(&mut e),
            RuleKind::Workspace => e.push(("workspace".into(), s(&self.workspace))),
            RuleKind::Layer => {}
        }
        self.push_props(e)
    }

    fn common_effects(&self, e: &mut Vec<(String, Value)>) {
        let s = |v: &str| Value::Str(v.trim().to_string());
        match self.state {
            FloatState::Float => e.push(("float".into(), Value::Bool(true))),
            FloatState::NoFloat => e.push(("float".into(), Value::Bool(false))),
            FloatState::Tile => e.push(("tile".into(), Value::Bool(true))),
            FloatState::Leave => {}
        }
        if !self.size_w.trim().is_empty() && !self.size_h.trim().is_empty() {
            e.push((
                "size".into(),
                Value::List(vec![s(&self.size_w), s(&self.size_h)]),
            ));
        }
        if !self.move_x.trim().is_empty() && !self.move_y.trim().is_empty() {
            e.push((
                "move".into(),
                Value::List(vec![s(&self.move_x), s(&self.move_y)]),
            ));
        }
        if self.center {
            e.push(("center".into(), Value::Bool(true)));
        }
        if !self.workspace.trim().is_empty() {
            let silent = if self.ws_silent { " silent" } else { "" };
            e.push((
                "workspace".into(),
                Value::Str(format!("{}{silent}", self.workspace.trim())),
            ));
        }
        if !self.monitor.trim().is_empty() {
            e.push(("monitor".into(), s(&self.monitor)));
        }
        if self.pin {
            e.push(("pin".into(), Value::Bool(true)));
        }
        if self.no_focus {
            e.push(("no_initial_focus".into(), Value::Bool(true)));
        }
    }

    fn push_props(&self, mut e: Vec<(String, Value)>) -> Vec<(String, Value)> {
        for (k, v) in &self.props {
            let k = k.trim();
            if !k.is_empty() {
                let value = self
                    .loaded_props
                    .iter()
                    .find(|(lk, lt, _)| lk == k && lt == v)
                    .map_or_else(|| parse_value(v), |(_, _, lv)| lv.clone());
                e.retain(|(key, _)| key != k);
                e.push((k.to_string(), value));
            }
        }
        e
    }
}

/// Left padding/right padding inside buttons, inputs and pick lists.
const PAD_X: f32 = 3.0 * style::U;
/// Line height of single-line inputs; with the vertical padding it adds up to CONTROL_H.
const INPUT_LINE: f32 = 18.0;
const INPUT_PAD_Y: f32 = (CONTROL_H - INPUT_LINE) / 2.0;
/// Width of the field/operator pick lists in a match row.
const KEY_W: f32 = 140.0;
/// Width of the "×" between the Size / Position inputs.
const PAIR_SEP: f32 = 20.0;

fn input<'a>(placeholder: &str, value: &str) -> TextInput<'a, Message> {
    text_input(placeholder, value)
        .size(TEXT)
        .line_height(Pixels(INPUT_LINE))
        .padding([INPUT_PAD_Y, PAD_X])
}

fn pick<'a, T, L, V>(
    options: L,
    selected: Option<V>,
    on_select: impl Fn(T) -> Message + 'a,
) -> PickList<'a, T, L, V, Message>
where
    T: ToString + PartialEq + Clone + 'a,
    L: Borrow<[T]> + 'a,
    V: Borrow<T> + 'a,
{
    pick_list(options, selected, on_select)
        .text_size(TEXT)
        .text_line_height(Pixels(INPUT_LINE))
        .padding([INPUT_PAD_Y, PAD_X])
}

fn btn_with<'a>(label: &'a str, on: Option<Message>, width: Length) -> Button<'a, Message> {
    button(single_line(label, TEXT).width(width).height(Fill).center())
        .width(width)
        .height(CONTROL_H)
        .padding([0.0, PAD_X])
        .style(style::action)
        .on_press_maybe(on)
}

fn btn<'a>(label: &'a str, on: Option<Message>) -> Button<'a, Message> {
    btn_with(label, on, Length::Shrink)
}

/// The trailing helper button of a form row ("Pick…" lists, "Copy from window…").
fn side_btn<'a>(label: &'a str, on: Option<Message>) -> Button<'a, Message> {
    btn_with(label, on, Length::Fixed(SIDE_W))
}

fn square_btn(label: &str, on: Message) -> Button<'_, Message> {
    button(text(label).size(TEXT).width(Fill).height(Fill).center())
        .width(CONTROL_H)
        .height(CONTROL_H)
        .padding(0)
        .style(style::action)
        .on_press(on)
}

fn help<'a>(content: impl text::IntoFragment<'a>) -> Text<'a> {
    text(content).size(TEXT_SMALL).style(text::secondary)
}

/// A form row; below LABEL_W + 360 the label goes above the control. An empty label
/// leaves the label column blank so the control lines up with the others.
fn field<'a>(
    narrow: bool,
    label: &'a str,
    control: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    match (narrow, label.is_empty()) {
        (true, true) => control.into(),
        (true, false) => column![single_line(label, TEXT), control.into()]
            .spacing(style::U)
            .into(),
        (false, _) => form_row(label, control),
    }
}

/// `main` with a trailing `side` control on the same line.
fn with_side<'a>(
    main: impl Into<Element<'a, Message>>,
    side: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    row![main.into(), side.into()]
        .spacing(GAP)
        .align_y(iced::Center)
        .into()
}

impl App {
    pub(super) fn view_editor(&self) -> Element<'_, Message> {
        if self.editor.is_none() || self.selected.and_then(|i| self.rules.get(i)).is_none() {
            return center(text("Select a rule, or add one.").style(text::secondary)).into();
        }
        responsive(move |size| self.editor_form(size.width)).into()
    }

    /// Keep the Group combo box's options (the other rules' groups) current. Typing a new
    /// name into the selected rule never changes them, so the box isn't rebuilt mid-edit.
    pub(super) fn sync_group_combo(&mut self) {
        let mut names: Vec<String> = self
            .rules
            .iter()
            .enumerate()
            .filter(|(i, r)| Some(*i) != self.selected && !r.group.is_empty())
            .map(|(_, r)| r.group.clone())
            .collect();
        names.sort();
        names.dedup();
        if self.group_combo.options() != names {
            self.group_combo = combo_box::State::new(names);
        }
    }

    fn editor_form(&self, width: f32) -> Element<'_, Message> {
        let (Some(ed), Some(i)) = (&self.editor, self.selected) else {
            return space().into();
        };
        let Some(rule) = self.rules.get(i) else {
            return space().into();
        };
        let avail = width - 2.0 * PAD;
        let two_columns = avail >= 2.0 * FORM_MAX_W + SECTION_GAP;
        let column_w = if two_columns {
            (avail - SECTION_GAP) / 2.0
        } else {
            avail
        }
        .min(FORM_MAX_W);
        let narrow = column_w - 2.0 * PAD < LABEL_W + 360.0;

        let mut left = Column::new()
            .spacing(SECTION_GAP)
            .push(self.header_card(ed, i, narrow));
        match ed.kind {
            RuleKind::Window => {
                left = left
                    .push(self.match_section(rule, "Applies to windows where…", narrow))
                    .push(self.common_section(ed, narrow));
            }
            RuleKind::Layer => {
                left = left.push(self.match_section(rule, "Applies to layers where…", narrow));
            }
            RuleKind::Workspace => {}
        }

        let mut right = Column::new()
            .spacing(SECTION_GAP)
            .push(self.other_section(ed, narrow));
        let findings = self.findings_for(i);
        if !findings.is_empty() {
            let mut list = Column::new().spacing(GAP);
            for f in findings {
                list = list.push(text(self.finding_line(&f.kind)).size(TEXT));
            }
            right = right.push(section("Rule checks", list));
        }

        let body: Element<'_, Message> = if two_columns {
            container(
                row![container(left).width(Fill), container(right).width(Fill)]
                    .spacing(SECTION_GAP),
            )
            .max_width(2.0 * FORM_MAX_W + SECTION_GAP)
            .width(Fill)
            .into()
        } else {
            container(left.push(right))
                .max_width(FORM_MAX_W)
                .width(Fill)
                .into()
        };
        scrollable(container(body).padding(PAD).center_x(Fill))
            .height(Fill)
            .into()
    }

    fn header_card<'a>(&'a self, ed: &'a Editor, i: usize, narrow: bool) -> Element<'a, Message> {
        let active = toggler(ed.enabled)
            .label("Active")
            .text_size(TEXT)
            .on_toggle(Message::Enabled);
        let mut actions = row![
            active,
            space().width(Fill),
            btn("Duplicate", Some(Message::Duplicate))
        ]
        .spacing(GAP)
        .align_y(iced::Center);
        if ed.kind == RuleKind::Window {
            actions = actions.push(btn(
                "Apply now",
                Some(Message::Inspect(inspect::Msg::ApplyNow(i))),
            ));
        }
        actions = actions.push(btn("Delete", Some(Message::Delete)));

        let mut card = Column::new().spacing(GAP).push(actions);
        let title = match ed.kind {
            RuleKind::Window => "Window rule",
            RuleKind::Layer => "Layer rule",
            RuleKind::Workspace => "Workspace rule",
        };
        if ed.kind == RuleKind::Workspace {
            let info = model::prop_info(RuleKind::Workspace, "workspace").map_or("", |p| p.help);
            card = card
                .push(field(
                    narrow,
                    "Workspaces",
                    with_side(
                        input("e.g. 2, name:web, special:music", &ed.workspace)
                            .on_input(Message::Workspace)
                            .width(Fill),
                        pick(
                            &self.workspace_names[..],
                            None::<String>,
                            Message::Workspace,
                        )
                        .placeholder("Pick…")
                        .width(SIDE_W),
                    ),
                ))
                .push(field(narrow, "", help(info)));
        } else {
            let name = input("optional, e.g. \"Steam games on workspace 2\"", &ed.name)
                .on_input(Message::Name)
                .width(Fill);
            card = card.push(field(narrow, "Name", name));
        }
        let group_row = combo_box(
            &self.group_combo,
            "none",
            Some(&ed.group).filter(|g| !g.is_empty()),
            Message::Group,
        )
        .on_input(Message::Group)
        .size(TEXT)
        .padding([INPUT_PAD_Y, PAD_X])
        .width(Fill);
        card = card.push(field(narrow, "Group", group_row));
        section(title, card)
    }

    fn finding_line(&self, kind: &FindingKind) -> String {
        let title = |i: usize| self.rules.get(i).map(Rule::title).unwrap_or_default();
        match kind {
            FindingKind::NoOpenMatch => "⚠ No open window matches this rule".into(),
            FindingKind::CatchAll => "⚠ Matches every window".into(),
            FindingKind::Overridden { by, key } => {
                format!("⚠ “{key}” is overridden by “{}”", title(*by))
            }
            FindingKind::Conflict { with, key } => {
                format!("⚠ “{key}” conflicts with “{}”", title(*with))
            }
        }
    }

    fn match_section<'a>(
        &'a self,
        rule: &'a Rule,
        title: &'a str,
        narrow: bool,
    ) -> Element<'a, Message> {
        let layer = rule.kind == RuleKind::Layer;
        let mut conds = Column::new().spacing(GAP);
        for (n, (i, m)) in rule
            .matches
            .iter()
            .enumerate()
            .filter(|(_, m)| m.is_pattern())
            .enumerate()
        {
            let invalid = m.mode == Mode::Regex && !model::valid_regex(&m.pattern());
            let placeholder = if m.mode == Mode::Regex {
                "regular expression (full match)"
            } else {
                "plain text — no regex needed"
            };
            let key: Element<'_, Message> = if layer {
                container(single_line("Namespace", TEXT))
                    .width(KEY_W)
                    .into()
            } else {
                pick(
                    &KEY_OPTS[..],
                    KEY_OPTS.iter().find(|k| k.0 == m.key).copied(),
                    move |k| Message::MatchKey(i, k),
                )
                .width(KEY_W)
                .into()
            };
            let mode = pick(&Mode::ALL[..], Some(m.mode), move |mode| {
                Message::MatchMode(i, mode)
            })
            .width(KEY_W);
            let value = input(placeholder, &m.text)
                .on_input(move |t| Message::MatchText(i, t))
                .width(Fill);
            let case = tooltip(
                checkbox(m.ignore_case)
                    .label("Any case")
                    .text_size(TEXT)
                    .on_toggle(move |c| Message::MatchCase(i, c)),
                container(text("Ignore upper/lower case differences").size(TEXT_SMALL))
                    .padding(style::U)
                    .style(container::rounded_box),
                tooltip::Position::Top,
            );
            let remove = square_btn("×", Message::MatchRemove(i));
            let grid: Element<'_, Message> = if narrow {
                column![
                    row![key, mode].spacing(GAP),
                    row![value, case, remove].spacing(GAP).align_y(iced::Center),
                ]
                .spacing(GAP)
                .into()
            } else {
                row![key, mode, value, case, remove]
                    .spacing(GAP)
                    .align_y(iced::Center)
                    .into()
            };
            let earlier_same_field = rule.matches[..i]
                .iter()
                .any(|o| o.is_pattern() && o.key == m.key);
            if earlier_same_field {
                conds = conds.push(field(narrow, "", help("or")));
            }
            conds = conds.push(field(narrow, if n == 0 { "Condition" } else { "" }, grid));
            if invalid {
                conds = conds.push(field(
                    narrow,
                    "",
                    text("⚠ invalid regex").size(TEXT_SMALL).style(text::danger),
                ));
            }
        }
        let preview: Element<'_, Message> = if layer {
            help("Layer rules have no live preview").into()
        } else {
            self.preview(rule)
        };
        section(
            title,
            column![
                conds,
                field(
                    narrow,
                    "",
                    btn("+ Add condition", Some(Message::MatchAdd))
                ),
                field(
                    narrow,
                    "",
                    column![
                        help("Different fields must all match; several conditions on the same field match if any one does."),
                        preview
                    ]
                    .spacing(GAP)
                ),
            ]
            .spacing(GAP),
        )
    }

    fn common_section<'a>(&'a self, ed: &'a Editor, narrow: bool) -> Element<'a, Message> {
        let geometry = |a: &'a str,
                        b: &'a str,
                        pa: &'static str,
                        pb: &'static str,
                        fa: fn(String) -> Message,
                        fb: fn(String) -> Message,
                        copy: Geometry| {
            let pair = row![
                input(pa, a).on_input(fa).width(Fill),
                container(single_line("×", TEXT)).center_x(PAIR_SEP),
                input(pb, b).on_input(fb).width(Fill),
            ]
            .align_y(iced::Center)
            .width(Fill);
            let copy = side_btn("Copy from window…", Some(Message::GeometryFromWindow(copy)));
            with_side(pair, copy)
        };
        let check = |label: &'a str, on: bool, msg: fn(bool) -> Message| {
            field(
                narrow,
                "",
                checkbox(on).label(label).text_size(TEXT).on_toggle(msg),
            )
        };
        section(
            "Common",
            column![
                field(
                    narrow,
                    "State",
                    pick(&FloatState::ALL[..], Some(ed.state), Message::State).width(Fill)
                ),
                field(
                    narrow,
                    "Size",
                    geometry(
                        &ed.size_w,
                        &ed.size_h,
                        "width",
                        "height",
                        Message::SizeW,
                        Message::SizeH,
                        Geometry::Size
                    )
                ),
                field(
                    narrow,
                    "Position",
                    geometry(
                        &ed.move_x,
                        &ed.move_y,
                        "x",
                        "y",
                        Message::MoveX,
                        Message::MoveY,
                        Geometry::Position
                    )
                ),
                check("Center on screen", ed.center, Message::Center),
                field(
                    narrow,
                    "Workspace",
                    with_side(input("e.g. 2, special:music", &ed.workspace)
                            .on_input(Message::Workspace)
                            .width(Fill),
                        pick(
                            &self.workspace_names[..],
                            None::<String>,
                            Message::Workspace
                        )
                        .placeholder("Pick…")
                        .width(SIDE_W),
                    )
                ),
                check(
                    "Silent (don't switch to it)",
                    ed.ws_silent,
                    Message::WsSilent
                ),
                field(
                    narrow,
                    "Monitor",
                    with_side(input("any", &ed.monitor)
                            .on_input(Message::Monitor)
                            .width(Fill),
                        pick(&self.monitor_names[..], None::<String>, Message::Monitor)
                            .placeholder("Pick…")
                            .width(SIDE_W),
                    )
                ),
                check(
                    "Pin (show on every workspace, floating only)",
                    ed.pin,
                    Message::Pin
                ),
                check("Don't focus when it opens", ed.no_focus, Message::NoFocus),
                field(
                    narrow,
                    "",
                    help("Sizes and positions accept pixels (800) or percentages (50%). Positions are relative to the monitor.")
                ),
            ]
            .spacing(GAP),
        )
    }

    fn other_section<'a>(&'a self, ed: &'a Editor, narrow: bool) -> Element<'a, Message> {
        let mut props = Column::new().spacing(GAP);
        for (i, (k, v)) in ed.props.iter().enumerate() {
            props = props.push(prop_row(ed.kind, i, k, v, narrow));
        }
        let mut options: Vec<String> = model::props_for(ed.kind)
            .iter()
            .map(|p| p.key)
            .filter(|k| {
                let common = ed.kind == RuleKind::Window && COMMON_KEYS.contains(k);
                let selector = ed.kind == RuleKind::Workspace && *k == "workspace";
                !common && !selector && !ed.props.iter().any(|(pk, _)| pk == k)
            })
            .map(str::to_string)
            .collect();
        options.sort();
        section(
            "Other properties",
            column![
                props,
                row![
                    pick(options, self.prop_pick.clone(), Message::PropPick)
                        .placeholder("Choose a property…")
                        .width(Fill),
                    side_btn("+ Add", self.prop_pick.as_ref().map(|_| Message::PropAdd)),
                ]
                .spacing(GAP)
                .align_y(iced::Center),
            ]
            .spacing(GAP),
        )
    }

    pub(super) fn preview(&self, rule: &Rule) -> Element<'_, Message> {
        if !rule.has_condition() {
            return help("Add at least one condition — a rule without one matches nothing.").into();
        }
        let hits: Vec<&Json> = self
            .clients
            .iter()
            .filter(|c| rule.matches_window(c))
            .collect();
        if hits.is_empty() {
            return help("No open window matches right now.").into();
        }
        let mut col = column![help(format!("Matches {} open window(s):", hits.len()))].spacing(2);
        for c in hits.iter().take(8) {
            col = col.push(help(format!(
                "• {} — {}",
                hypr::str_field(c, "class"),
                hypr::str_field(c, "title")
            )));
        }
        if hits.len() > 8 {
            col = col.push(help(format!("…and {} more", hits.len() - 8)));
        }
        col.into()
    }
}

/// One "Other properties" row: the key as the label, a typed widget with help for known
/// keys, an editable key with a warning for unknown ones.
fn prop_row<'a>(
    kind: RuleKind,
    i: usize,
    k: &'a str,
    v: &'a str,
    narrow: bool,
) -> Element<'a, Message> {
    let remove = square_btn("×", Message::PropRemove(i));
    let Some(info) = model::prop_info(kind, k) else {
        let key = input("property", k)
            .on_input(move |s| Message::PropKey(i, s))
            .width(if narrow { Fill } else { Length::Fixed(LABEL_W) });
        let value = input("value", v)
            .on_input(move |s| Message::PropValue(i, s))
            .width(Fill);
        let warning = text("⚠ unknown property — Hyprland will reject it")
            .size(TEXT_SMALL)
            .style(text::danger);
        return if narrow {
            column![
                key,
                row![value, remove].spacing(GAP).align_y(iced::Center),
                warning
            ]
            .spacing(GAP)
            .into()
        } else {
            column![
                row![key, value, remove].spacing(GAP).align_y(iced::Center),
                field(false, "", warning),
            ]
            .spacing(GAP)
            .into()
        };
    };
    let hint = match info.kind {
        Kind::Bool => "true / false",
        Kind::Int => "number",
        Kind::Float => "decimal, e.g. 0.5",
        Kind::Pair => "{ a, b }",
        Kind::Str => "value",
    };
    let edit = || {
        input(hint, v)
            .on_input(move |s| Message::PropValue(i, s))
            .width(Fill)
            .into()
    };
    let mut warn = false;
    let value: Element<'a, Message> = match info.kind {
        Kind::Bool if v == "true" || v == "false" => checkbox(v == "true")
            .on_toggle(move |b| Message::PropValue(i, b.to_string()))
            .into(),
        _ if !info.choices.is_empty() => match info.choices.iter().find(|c| **c == v) {
            Some(current) => pick(info.choices, Some(*current), move |c: &str| {
                Message::PropValue(i, c.to_string())
            })
            .width(Fill)
            .into(),
            None => edit(),
        },
        Kind::Int => {
            warn = v.trim().parse::<i64>().is_err();
            edit()
        }
        Kind::Float => {
            warn = v.trim().parse::<f64>().is_err();
            edit()
        }
        _ => edit(),
    };
    let label = tooltip(
        container(single_line(k, TEXT).font(MONO))
            .width(if narrow { Fill } else { Length::Fixed(LABEL_W) })
            .height(CONTROL_H)
            .align_y(iced::Center)
            .clip(true),
        container(text(info.help).size(TEXT_SMALL))
            .padding(style::U)
            .style(container::rounded_box),
        tooltip::Position::Top,
    );
    let mut control = column![
        row![container(value).width(Fill), remove]
            .spacing(GAP)
            .align_y(iced::Center),
    ]
    .spacing(style::U);
    if warn {
        control = control.push(text("not a number").size(TEXT_SMALL).style(text::danger));
    }
    control = control.push(help(info.help));
    if narrow {
        column![label, control].spacing(style::U).into()
    } else {
        row![label, control].spacing(GAP).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_keeps_effects_the_widgets_cant_show() {
        let s = |v: &str| Value::Str(v.into());
        let effects = vec![
            ("float".to_string(), Value::Bool(true)),
            ("size".to_string(), s("1000 700")),
            (
                "move".to_string(),
                Value::List(vec![s("10"), Value::Int(20)]),
            ),
            ("center".to_string(), Value::Bool(false)),
            ("opacity".to_string(), Value::Float(1.0)),
            ("tag".to_string(), s("a, b")),
            (
                "border_color".to_string(),
                Value::Map(vec![("active".into(), s("rgb(ff0000)"))]),
            ),
        ];
        let rule = Rule {
            effects: effects.clone(),
            ..Default::default()
        };
        let mut ed = Editor::from_rule(&rule);
        ed.name = "renamed".into();
        let mut after = ed.effects();
        after.sort_by(|a, b| a.0.cmp(&b.0));
        let mut want = effects;
        // The widgets write sizes and positions as strings, which Hyprland reads the same
        want[2].1 = Value::List(vec![s("10"), s("20")]);
        want.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(after, want);
    }

    #[test]
    fn workspace_rule_keeps_selector_and_props() {
        let rule = Rule {
            kind: RuleKind::Workspace,
            effects: vec![
                ("workspace".into(), Value::Str("w[tv1]".into())),
                ("gaps_out".into(), Value::Int(0)),
            ],
            ..Default::default()
        };
        let ed = Editor::from_rule(&rule);
        assert_eq!(ed.workspace, "w[tv1]");
        assert_eq!(ed.props.len(), 1);
        assert_eq!(ed.effects(), rule.effects);
    }

    #[test]
    fn layer_rule_keeps_all_effects() {
        let rule = Rule {
            kind: RuleKind::Layer,
            effects: vec![
                ("blur".into(), Value::Bool(true)),
                ("workspace".into(), Value::Str("2".into())),
                ("ignore_alpha".into(), Value::Float(0.5)),
            ],
            ..Default::default()
        };
        assert_eq!(Editor::from_rule(&rule).effects(), rule.effects);
    }

    #[test]
    fn bool_prop_text_becomes_a_bool() {
        let rule = Rule {
            effects: vec![("no_blur".into(), Value::Bool(false))],
            ..Default::default()
        };
        let mut ed = Editor::from_rule(&rule);
        ed.props[0].1 = "true".into();
        assert_eq!(ed.effects(), vec![("no_blur".into(), Value::Bool(true))]);
    }
}
