//! iced user interface.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use iced::keyboard::{self, Key, key::Named};
use iced::widget::{
    Column, button, center, checkbox, column, container, opaque, pick_list, progress_bar, row,
    scrollable, space, stack, text, text_input, tooltip,
};
use iced::{Color, Element, Fill, Font, Subscription, Task, Theme, window};
use serde_json::Value as Json;

use crate::audio_ui::{self, AudioTab};
use crate::model::{self, Kind, MatchField, Mode, PATTERN_KEYS, Rule, Value, effect_kind};
use crate::settings::{self, Settings, ThemePref};
use crate::update::{self, InstallKind, Progress, Update};
use crate::{hypr, luaio};

/// Effects edited by dedicated widgets in the "Common" group; the rest go in the table.
const COMMON_KEYS: [&str; 9] = [
    "float",
    "tile",
    "size",
    "move",
    "center",
    "workspace",
    "monitor",
    "pin",
    "no_initial_focus",
];

const APP_TITLE: &str = "Hyprland Windows Rules";

// ---- value <-> text for the free-form property table ------------------------------------

fn parse_value(text: &str) -> Value {
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

fn format_value(v: &Value) -> String {
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
pub struct KeyOpt(&'static str);

impl std::fmt::Display for KeyOpt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(model::match_key_label(self.0))
    }
}

const KEY_OPTS: [KeyOpt; 7] = [
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
struct Editor {
    name: String,
    enabled: bool,
    state: FloatState,
    size_w: String,
    size_h: String,
    move_x: String,
    move_y: String,
    center: bool,
    workspace: String,
    ws_silent: bool,
    monitor: String,
    pin: bool,
    no_focus: bool,
    props: Vec<(String, String)>,
}

impl Editor {
    fn from_rule(rule: &Rule) -> Self {
        let pair = |key: &str| match rule.effect(key) {
            Some(Value::List(v)) if v.len() == 2 => (v[0].plain(), v[1].plain()),
            _ => Default::default(),
        };
        let (size_w, size_h) = pair("size");
        let (move_x, move_y) = pair("move");
        let state = match (rule.effect("float"), rule.effect("tile")) {
            (Some(Value::Bool(true)), _) => FloatState::Float,
            (Some(Value::Bool(false)), _) => FloatState::NoFloat,
            (_, Some(v)) if v.truthy() => FloatState::Tile,
            _ => FloatState::Leave,
        };
        let flag = |key: &str| rule.effect(key).is_some_and(Value::truthy);
        let ws = rule
            .effect("workspace")
            .map(Value::plain)
            .unwrap_or_default();
        let ws_silent = ws.ends_with(" silent");
        Self {
            name: rule.name.clone(),
            enabled: rule.enabled,
            state,
            size_w,
            size_h,
            move_x,
            move_y,
            center: flag("center"),
            workspace: ws.strip_suffix(" silent").unwrap_or(&ws).to_string(),
            ws_silent,
            monitor: rule.effect("monitor").map(Value::plain).unwrap_or_default(),
            pin: flag("pin"),
            no_focus: flag("no_initial_focus"),
            props: rule
                .effects
                .iter()
                .filter(|(k, _)| !COMMON_KEYS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), format_value(v)))
                .collect(),
        }
    }

    fn effects(&self) -> Vec<(String, Value)> {
        let mut e: Vec<(String, Value)> = Vec::new();
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
        for (k, v) in &self.props {
            let k = k.trim();
            if !k.is_empty() {
                e.retain(|(key, _)| key != k);
                e.push((k.to_string(), parse_value(v)));
            }
        }
        e
    }
}

enum Modal {
    Message {
        title: String,
        body: String,
        buttons: Vec<(String, Message)>,
    },
    Picker {
        clients: Vec<Json>,
        selected: Option<usize>,
        use_title: bool,
        copy_geom: bool,
        copy_ws: bool,
        /// Copy the picked window's size/position into the selected rule instead
        /// of creating a new rule.
        for_current: Option<Geometry>,
    },
    Import {
        path: PathBuf,
        rules: Vec<Rule>,
        checked: Vec<bool>,
        can_comment: bool,
        comment_out: bool,
        warning: Option<String>,
    },
    Settings {
        rules: String,
        hook: String,
        reload: bool,
        check_updates: bool,
    },
}

pub struct App {
    settings: Settings,
    rules: Vec<Rule>,
    selected: Option<usize>,
    editor: Option<Editor>,
    dirty: bool,
    clients: Vec<Json>,
    monitors: Vec<Json>,
    monitor_names: Vec<String>,
    live_workspaces: Vec<String>,
    /// Existing workspaces plus any the rules mention, for the workspace drop-down.
    workspace_names: Vec<String>,
    prop_options: Vec<String>,
    prop_pick: Option<String>,
    search: String,
    note: String,
    modal: Option<Modal>,
    install_kind: InstallKind,
    update: UpdateState,
    tab: Tab,
    audio: AudioTab,
}

/// Which part of a window's geometry "Copy from window…" fills in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Geometry {
    Size,
    Position,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Rules,
    Audio,
}

enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available(Update),
    Downloading {
        version: String,
        progress: Arc<Progress>,
    },
    Ready(String),
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum Message {
    // toolbar
    AddRule,
    FromWindow,
    GeometryFromWindow(Geometry),
    Duplicate,
    Delete,
    DeleteConfirmed,
    Move(isize),
    Import,
    ImportFrom(Option<PathBuf>),
    Refresh,
    OpenSettings,
    Theme(ThemePref),
    Save,
    LoadRules,
    InstallHook,
    InstallHookConfirmed,
    Reloaded(Result<Vec<String>, String>),
    // list
    Search(String),
    Select(usize),
    // editor
    Name(String),
    Enabled(bool),
    State(FloatState),
    SizeW(String),
    SizeH(String),
    MoveX(String),
    MoveY(String),
    Center(bool),
    Workspace(String),
    WsSilent(bool),
    Monitor(String),
    Pin(bool),
    NoFocus(bool),
    MatchAdd,
    MatchKey(usize, KeyOpt),
    MatchMode(usize, Mode),
    MatchText(usize, String),
    MatchCase(usize, bool),
    MatchRemove(usize),
    PropPick(String),
    PropAdd,
    PropKey(usize, String),
    PropValue(usize, String),
    PropRemove(usize),
    // modals
    CloseModal,
    PickerSelect(usize),
    PickerUseTitle(bool),
    PickerCopyGeom(bool),
    PickerCopyWs(bool),
    PickerOk,
    ImportToggle(usize, bool),
    ImportCommentOut(bool),
    ImportOk,
    SettingsRules(String),
    SettingsHook(String),
    SettingsReload(bool),
    BrowseRules,
    BrowseHook,
    BrowsedRules(Option<PathBuf>),
    BrowsedHook(Option<PathBuf>),
    SettingsOk,
    // window
    CloseRequested,
    CloseSave,
    Exit,
    // updates
    CheckUpdates,
    UpdateChecked {
        manual: bool,
        result: Result<Option<Update>, String>,
    },
    InstallUpdate,
    UpdateTick,
    UpdateApplied(Result<(), String>),
    Restart,
    RestartSave,
    RestartNow,
    SettingsCheckUpdates(bool),
    // tabs
    Tab(Tab),
    Audio(audio_ui::Msg),
}

pub fn run() -> iced::Result {
    iced::application(App::boot, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        .window(window::Settings {
            size: iced::Size::new(1200.0, 760.0),
            platform_specific: window::settings::PlatformSpecific {
                application_id: "hypr-rules".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .exit_on_close_request(false)
        .run()
}

fn shortcut(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };
    match key.as_ref() {
        Key::Character("s") if modifiers.command() => Some(Message::Save),
        Key::Character("n") if modifiers.command() => Some(Message::AddRule),
        Key::Character("w") if modifiers.command() => Some(Message::FromWindow),
        Key::Character("d") if modifiers.command() => Some(Message::Duplicate),
        Key::Named(Named::Delete) => Some(Message::Delete),
        Key::Named(Named::ArrowUp) if modifiers.alt() => Some(Message::Move(-1)),
        Key::Named(Named::ArrowDown) if modifiers.alt() => Some(Message::Move(1)),
        Key::Named(Named::F5) => Some(Message::Refresh),
        Key::Named(Named::Escape) => Some(Message::CloseModal),
        _ => None,
    }
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let mut prop_options: Vec<String> = model::EFFECTS
            .iter()
            .map(|(k, _)| k.to_string())
            .filter(|k| !COMMON_KEYS.contains(&k.as_str()))
            .collect();
        prop_options.sort();
        let mut app = App {
            settings: Settings::load(),
            rules: Vec::new(),
            selected: None,
            editor: None,
            dirty: false,
            clients: Vec::new(),
            monitors: Vec::new(),
            monitor_names: Vec::new(),
            live_workspaces: Vec::new(),
            workspace_names: Vec::new(),
            prop_options,
            prop_pick: None,
            search: String::new(),
            note: String::new(),
            modal: None,
            install_kind: update::install_kind(),
            update: UpdateState::Idle,
            tab: Tab::Rules,
            audio: AudioTab::default_closed(),
        };
        app.refresh_windows();
        app.load_rules();
        if app.modal.is_none() && !app.settings.rules().exists() {
            app.offer_first_import();
        }
        let task = if app.install_kind != InstallKind::Source && app.settings.check_updates {
            app.check_updates(false)
        } else {
            Task::none()
        };
        let (audio, audio_task) = AudioTab::new();
        app.audio = audio;
        (app, Task::batch([task, audio_task.map(Message::Audio)]))
    }

    fn title(&self) -> String {
        format!(
            "{}{APP_TITLE}",
            if self.dirty || self.audio.dirty {
                "• "
            } else {
                ""
            }
        )
    }

    /// None lets iced follow the system light/dark preference.
    fn theme(&self) -> Option<Theme> {
        match self.settings.theme {
            ThemePref::System => None,
            ThemePref::Light => Some(Theme::Light),
            ThemePref::Dark => Some(Theme::Dark),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        // Redraws the download progress while an update downloads
        let tick = match self.update {
            UpdateState::Downloading { .. } => {
                iced::time::every(Duration::from_millis(150)).map(|_| Message::UpdateTick)
            }
            _ => Subscription::none(),
        };
        Subscription::batch([
            keyboard::listen().filter_map(shortcut),
            window::close_requests().map(|_| Message::CloseRequested),
            tick,
        ])
    }

    fn check_updates(&mut self, manual: bool) -> Task<Message> {
        if manual {
            self.update = UpdateState::Checking;
        }
        let kind = self.install_kind.clone();
        Task::perform(update::check(kind), move |r| Message::UpdateChecked {
            manual,
            result: r.map_err(|e| format!("{e:#}")),
        })
    }

    fn restart(&mut self) -> Task<Message> {
        match update::relaunch(&self.install_kind) {
            Ok(()) => iced::exit(),
            Err(err) => {
                self.info("Restart", format!("{err:#}"));
                Task::none()
            }
        }
    }

    fn info(&mut self, title: &str, body: impl Into<String>) {
        self.modal = Some(Modal::Message {
            title: title.into(),
            body: body.into(),
            buttons: vec![("OK".into(), Message::CloseModal)],
        });
    }

    // -- data

    fn load_rules(&mut self) {
        let path = self.settings.rules();
        self.rules.clear();
        if path.exists() {
            match luaio::evaluate_rules(&path) {
                Ok(res) => {
                    self.rules = res.rules;
                    if let Some(err) = res.error {
                        self.info(
                            "Rules file",
                            format!("The rules file has an error:\n\n{err}"),
                        );
                    }
                }
                Err(err) => self.info("Rules file", err),
            }
        }
        self.dirty = false;
        self.select(if self.rules.is_empty() { None } else { Some(0) });
    }

    fn offer_first_import(&mut self) {
        for file in settings::import_candidates(&self.settings.hook()) {
            let Ok(src) = std::fs::read_to_string(&file) else {
                continue;
            };
            let count = luaio::find_window_rule_calls(&src).len();
            if count > 0 {
                self.modal = Some(Modal::Message {
                    title: "Import existing rules?".into(),
                    body: format!(
                        "{} already has {count} window rule(s).\n\nImport them so you can manage them here?",
                        file.display()
                    ),
                    buttons: vec![
                        ("Import".into(), Message::ImportFrom(Some(file))),
                        ("Not now".into(), Message::CloseModal),
                    ],
                });
                return;
            }
        }
    }

    fn refresh_windows(&mut self) {
        match hypr::clients().and_then(|c| Ok((c, hypr::monitors()?))) {
            Ok((clients, monitors)) => {
                self.clients = clients;
                self.monitors = monitors;
            }
            Err(err) => {
                self.clients.clear();
                self.monitors.clear();
                self.note = format!("Can't reach Hyprland: {err}");
            }
        }
        self.monitor_names = self
            .monitors
            .iter()
            .map(|m| hypr::str_field(m, "name").to_string())
            .collect();
        self.live_workspaces = hypr::workspaces().unwrap_or_default();
        self.refresh_workspace_names();
    }

    /// Numbered workspaces first, then named, then special ones.
    fn refresh_workspace_names(&mut self) {
        let from_rules = self
            .rules
            .iter()
            .filter_map(|r| r.effect("workspace"))
            .map(|v| {
                let ws = v.plain();
                ws.strip_suffix(" silent").unwrap_or(&ws).trim().to_string()
            });
        let mut names: Vec<String> = self
            .live_workspaces
            .iter()
            .cloned()
            .chain(from_rules)
            .filter(|n| !n.is_empty())
            .collect();
        names.sort_by_key(|n| match (n.starts_with("special:"), n.parse::<i64>()) {
            (false, Ok(num)) => (0, num, String::new()),
            (false, Err(_)) => (1, 0, n.clone()),
            (true, _) => (2, 0, n.clone()),
        });
        names.dedup();
        self.workspace_names = names;
    }

    fn select(&mut self, idx: Option<usize>) {
        self.refresh_workspace_names();
        self.selected = idx.filter(|&i| i < self.rules.len());
        self.editor = self.selected.map(|i| Editor::from_rule(&self.rules[i]));
    }

    fn insert(&mut self, rule: Rule) {
        let idx = self.selected.map_or(self.rules.len(), |i| i + 1);
        self.rules.insert(idx, rule);
        self.dirty = true;
        self.search.clear();
        self.select(Some(idx));
    }

    fn current(&mut self) -> Option<&mut Rule> {
        self.selected.and_then(|i| self.rules.get_mut(i))
    }

    /// Editor fields -> rule effects.
    fn sync(&mut self) {
        let Some(ed) = &self.editor else { return };
        let (name, enabled, effects) = (ed.name.trim().to_string(), ed.enabled, ed.effects());
        if let Some(rule) = self.current() {
            rule.name = name;
            rule.enabled = enabled;
            rule.effects = effects;
        }
        self.dirty = true;
    }

    fn edit_match(&mut self, i: usize, f: impl FnOnce(&mut MatchField)) {
        if let Some(m) = self.current().and_then(|r| r.matches.get_mut(i)) {
            f(m);
            m.raw = None; // edited: regenerate the pattern from mode + text
        }
        self.dirty = true;
    }

    /// Write the rules file. Asks to set up the hook if Hyprland doesn't load the file yet.
    fn save(&mut self, reload: bool) -> Result<Task<Message>, ()> {
        model::unique_names(&mut self.rules);
        if let Err(err) = luaio::write_rules(&self.settings.rules(), &self.rules) {
            self.info("Save failed", err.to_string());
            return Err(());
        }
        self.dirty = false;
        self.select(self.selected);
        self.note.clear();
        if !settings::hook_installed(&self.settings.hook(), &self.settings.rules()) {
            self.ask_install_hook();
            Ok(Task::none())
        } else if reload {
            Ok(self.reload_hyprland())
        } else {
            Ok(Task::none())
        }
    }

    /// Hyprland reloads by itself when a watched file changes; only ask it explicitly when
    /// autoreload is off. Config errors are checked once the reload has had time to happen.
    fn reload_hyprland(&mut self) -> Task<Message> {
        if !hypr::autoreload_enabled()
            && let Err(err) = hypr::reload()
        {
            self.info(
                "Reload",
                format!("Saved, but reloading Hyprland failed:\n\n{err}"),
            );
            return Task::none();
        }
        Task::perform(
            async {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                hypr::config_errors()
            },
            Message::Reloaded,
        )
    }

    fn ask_install_hook(&mut self) {
        let (hook, rules) = (self.settings.hook(), self.settings.rules());
        self.modal = Some(Modal::Message {
            title: "Load rules in Hyprland".into(),
            body: format!(
                "Add this to {}?\n\n{}",
                hook.display(),
                settings::hook_text(&hook, &rules)
            ),
            buttons: vec![
                ("Add line".into(), Message::InstallHookConfirmed),
                ("Cancel".into(), Message::CloseModal),
            ],
        });
    }

    fn import_from(&mut self, path: &Path) {
        let same = |a: &Path, b: &Path| {
            a.canonicalize()
                .ok()
                .zip(b.canonicalize().ok())
                .is_some_and(|(a, b)| a == b)
        };
        if same(path, &self.settings.rules()) {
            return self.info("Import", "That's the file this app already manages.");
        }
        let res = match luaio::evaluate_rules(path) {
            Ok(res) => res,
            Err(err) => return self.info("Import failed", err),
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if res.rules.is_empty() {
            let extra = res.error.map(|e| format!("\n\n{e}")).unwrap_or_default();
            return self.info("Import", format!("No window rules found in {name}.{extra}"));
        }
        let spans = std::fs::read_to_string(path)
            .map(|s| luaio::find_window_rule_calls(&s).len())
            .unwrap_or(0);
        let can_comment = spans == res.rules.len() && res.error.is_none();
        let warning = res.error.map(|e| {
            format!("{name} stopped with an error part-way; rules before it were still read:\n{e}")
        });
        self.modal = Some(Modal::Import {
            path: path.to_path_buf(),
            checked: vec![true; res.rules.len()],
            rules: res.rules,
            can_comment,
            comment_out: can_comment,
            warning,
        });
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        // While a dialog is open, keyboard shortcuts only close it
        if self.modal.is_some()
            && matches!(
                message,
                Message::Save
                    | Message::AddRule
                    | Message::FromWindow
                    | Message::Duplicate
                    | Message::Delete
                    | Message::Move(_)
                    | Message::Refresh
            )
        {
            return Task::none();
        }
        if self.tab == Tab::Audio
            && matches!(
                message,
                Message::AddRule
                    | Message::FromWindow
                    | Message::Duplicate
                    | Message::Delete
                    | Message::Move(_)
            )
        {
            return Task::none();
        }
        match message {
            Message::Tab(tab) => self.tab = tab,
            Message::Audio(msg) => return self.audio.update(msg).map(Message::Audio),
            Message::AddRule => {
                self.insert(Rule {
                    matches: vec![MatchField::new("class")],
                    ..Default::default()
                });
            }
            Message::FromWindow => {
                self.refresh_windows();
                if self.clients.is_empty() {
                    self.info("No windows", "No open windows were found.");
                } else {
                    self.modal = Some(Modal::Picker {
                        clients: self.clients.clone(),
                        selected: Some(0),
                        use_title: false,
                        copy_geom: true,
                        copy_ws: false,
                        for_current: None,
                    });
                }
            }
            Message::GeometryFromWindow(part) => {
                self.refresh_windows();
                let Some(rule) = self.selected.and_then(|i| self.rules.get(i)) else {
                    return Task::none();
                };
                if self.clients.is_empty() {
                    self.info("No windows", "No open windows were found.");
                } else {
                    // Start on a window this rule matches, if one is open
                    let selected = self
                        .clients
                        .iter()
                        .position(|c| rule.matches_window(c))
                        .or(Some(0));
                    self.modal = Some(Modal::Picker {
                        clients: self.clients.clone(),
                        selected,
                        use_title: false,
                        copy_geom: true,
                        copy_ws: false,
                        for_current: Some(part),
                    });
                }
            }
            Message::Duplicate => {
                if let Some(i) = self.selected {
                    let mut dup = self.rules[i].clone();
                    if !dup.name.is_empty() {
                        dup.name = format!("{} (copy)", dup.name);
                    }
                    self.insert(dup);
                }
            }
            Message::Delete => {
                if let Some(i) = self.selected {
                    self.modal = Some(Modal::Message {
                        title: "Delete rule".into(),
                        body: format!("Delete “{}”?", self.rules[i].title()),
                        buttons: vec![
                            ("Delete".into(), Message::DeleteConfirmed),
                            ("Cancel".into(), Message::CloseModal),
                        ],
                    });
                }
            }
            Message::DeleteConfirmed => {
                self.modal = None;
                if let Some(i) = self.selected {
                    self.rules.remove(i);
                    self.dirty = true;
                    self.select(if self.rules.is_empty() {
                        None
                    } else {
                        Some(i.min(self.rules.len() - 1))
                    });
                }
            }
            Message::Move(delta) => {
                if let Some(i) = self.selected {
                    let j = i as isize + delta;
                    if j >= 0 && (j as usize) < self.rules.len() {
                        self.rules.swap(i, j as usize);
                        self.dirty = true;
                        self.selected = Some(j as usize);
                    }
                }
            }
            Message::Import => {
                let dir = self
                    .settings
                    .hook()
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(settings::home);
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Import from")
                            .add_filter("Lua", &["lua"])
                            .set_directory(dir)
                            .pick_file()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    Message::ImportFrom,
                );
            }
            Message::ImportFrom(path) => {
                self.modal = None;
                if let Some(path) = path {
                    self.import_from(&path);
                }
            }
            Message::Refresh => self.refresh_windows(),
            Message::Theme(pref) => {
                self.settings.theme = pref;
                if let Err(err) = self.settings.save() {
                    self.info("Settings", format!("Couldn't save settings:\n\n{err}"));
                }
            }
            Message::OpenSettings => {
                self.modal = Some(Modal::Settings {
                    rules: self.settings.rules_path.clone(),
                    hook: self.settings.hook_file.clone(),
                    reload: self.settings.reload_on_save,
                    check_updates: self.settings.check_updates,
                });
            }
            Message::Save if self.tab == Tab::Audio => {
                self.modal = None;
                return self.audio.save().map(Message::Audio);
            }
            Message::Save => {
                self.modal = None;
                let reload = self.settings.reload_on_save;
                return self.save(reload).unwrap_or_else(|_| Task::none());
            }
            Message::LoadRules => {
                self.modal = None;
                self.load_rules();
            }
            Message::InstallHook => self.ask_install_hook(),
            Message::InstallHookConfirmed => {
                self.modal = None;
                let (hook, rules) = (self.settings.hook(), self.settings.rules());
                if !rules.exists()
                    && let Err(err) = luaio::write_rules(&rules, &self.rules)
                {
                    self.info("Save failed", err.to_string());
                    return Task::none();
                }
                if let Err(err) = settings::install_hook(&hook, &rules) {
                    self.info(
                        "Load rules in Hyprland",
                        format!("Couldn't update {}:\n\n{err}", hook.display()),
                    );
                    return Task::none();
                }
                return self.reload_hyprland();
            }
            Message::Reloaded(Ok(errors)) if !errors.is_empty() => {
                self.info("Hyprland config errors", errors.join("\n"))
            }
            Message::Reloaded(Ok(_)) => {
                self.note = "Hyprland reloaded (rules apply to newly opened windows)".into()
            }
            Message::Reloaded(Err(err)) => {
                self.note = format!("Couldn't check Hyprland for config errors: {err}")
            }
            Message::Search(s) => self.search = s,
            Message::Select(i) => self.select(Some(i)),

            Message::Name(v) => self.edit(|e| e.name = v),
            Message::Enabled(v) => self.edit(|e| e.enabled = v),
            Message::State(v) => self.edit(|e| e.state = v),
            Message::SizeW(v) => self.edit(|e| e.size_w = v),
            Message::SizeH(v) => self.edit(|e| e.size_h = v),
            Message::MoveX(v) => self.edit(|e| e.move_x = v),
            Message::MoveY(v) => self.edit(|e| e.move_y = v),
            Message::Center(v) => self.edit(|e| e.center = v),
            Message::Workspace(v) => self.edit(|e| e.workspace = v),
            Message::WsSilent(v) => self.edit(|e| e.ws_silent = v),
            Message::Monitor(v) => self.edit(|e| e.monitor = v),
            Message::Pin(v) => self.edit(|e| e.pin = v),
            Message::NoFocus(v) => self.edit(|e| e.no_focus = v),
            Message::MatchAdd => {
                if let Some(rule) = self.current() {
                    let key = PATTERN_KEYS
                        .iter()
                        .find(|k| !rule.matches.iter().any(|m| m.key == **k))
                        .unwrap_or(&"title");
                    rule.matches.push(MatchField::new(key));
                    self.dirty = true;
                }
            }
            Message::MatchKey(i, k) => self.edit_match(i, |m| m.key = k.0.into()),
            Message::MatchMode(i, mode) => self.edit_match(i, |m| m.mode = mode),
            Message::MatchText(i, t) => self.edit_match(i, |m| m.text = t),
            Message::MatchCase(i, c) => self.edit_match(i, |m| m.ignore_case = c),
            Message::MatchRemove(i) => {
                if let Some(rule) = self.current() {
                    if i < rule.matches.len() {
                        rule.matches.remove(i);
                    }
                    self.dirty = true;
                }
            }
            Message::PropPick(k) => self.prop_pick = Some(k),
            Message::PropAdd => {
                if let Some(key) = self.prop_pick.clone() {
                    let default = match effect_kind(&key) {
                        Some(Kind::Bool) => "true",
                        Some(Kind::Int) => "0",
                        Some(Kind::Float) => "0.5",
                        Some(Kind::Pair) => "{ \"50%\", \"50%\" }",
                        _ => "",
                    };
                    self.edit(|e| e.props.push((key, default.into())));
                }
            }
            Message::PropKey(i, k) => self.edit(|e| e.props[i].0 = k),
            Message::PropValue(i, v) => self.edit(|e| e.props[i].1 = v),
            Message::PropRemove(i) => self.edit(|e| {
                e.props.remove(i);
            }),

            Message::CloseModal => self.modal = None,
            Message::PickerSelect(i) => self.picker(|p| *p.1 = Some(i)),
            Message::PickerUseTitle(v) => self.picker(|p| *p.2 = v),
            Message::PickerCopyGeom(v) => self.picker(|p| *p.3 = v),
            Message::PickerCopyWs(v) => self.picker(|p| *p.4 = v),
            Message::PickerOk => self.picker_ok(),
            Message::ImportToggle(i, v) => {
                if let Some(Modal::Import { checked, .. }) = &mut self.modal {
                    checked[i] = v;
                }
            }
            Message::ImportCommentOut(v) => {
                if let Some(Modal::Import { comment_out, .. }) = &mut self.modal {
                    *comment_out = v;
                }
            }
            Message::ImportOk => return self.import_ok(),
            Message::SettingsRules(v) => self.settings_dlg(|r, _, _| *r = v),
            Message::SettingsHook(v) => self.settings_dlg(|_, h, _| *h = v),
            Message::SettingsReload(v) => self.settings_dlg(|_, _, r| *r = v),
            Message::BrowseRules => {
                let current = self.settings.rules();
                return Task::perform(
                    async move {
                        let mut dlg = rfd::AsyncFileDialog::new()
                            .set_title("Rules file")
                            .add_filter("Lua", &["lua"]);
                        if let Some(dir) = current.parent() {
                            dlg = dlg.set_directory(dir);
                        }
                        if let Some(name) = current.file_name() {
                            dlg = dlg.set_file_name(name.to_string_lossy());
                        }
                        dlg.save_file().await.map(|h| h.path().to_path_buf())
                    },
                    Message::BrowsedRules,
                );
            }
            Message::BrowseHook => {
                let dir = self
                    .settings
                    .hook()
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(settings::home);
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Config file")
                            .add_filter("Lua", &["lua"])
                            .set_directory(dir)
                            .pick_file()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    Message::BrowsedHook,
                );
            }
            Message::BrowsedRules(Some(p)) => {
                self.settings_dlg(|r, _, _| *r = p.to_string_lossy().into_owned())
            }
            Message::BrowsedHook(Some(p)) => {
                self.settings_dlg(|_, h, _| *h = p.to_string_lossy().into_owned())
            }
            Message::BrowsedRules(None) | Message::BrowsedHook(None) => {}
            Message::SettingsOk => return self.settings_ok(),

            Message::CloseRequested => {
                if self.dirty || self.audio.dirty {
                    self.modal = Some(Modal::Message {
                        title: "Unsaved changes".into(),
                        body: "Save changes before closing?".into(),
                        buttons: vec![
                            ("Save".into(), Message::CloseSave),
                            ("Discard".into(), Message::Exit),
                            ("Cancel".into(), Message::CloseModal),
                        ],
                    });
                } else {
                    return iced::exit();
                }
            }
            Message::CloseSave => {
                self.modal = None;
                if self.audio.dirty {
                    // Only the files matter on the way out; the follow-up refresh is dropped
                    let _ = self.audio.save();
                    if self.audio.dirty {
                        self.tab = Tab::Audio;
                        return Task::none();
                    }
                }
                if !self.dirty {
                    return iced::exit();
                }
                model::unique_names(&mut self.rules);
                match luaio::write_rules(&self.settings.rules(), &self.rules) {
                    Ok(_) => return iced::exit(),
                    Err(err) => self.info("Save failed", err.to_string()),
                }
            }
            Message::Exit => return iced::exit(),

            Message::CheckUpdates => {
                self.modal = None;
                return self.check_updates(true);
            }
            Message::UpdateChecked { manual, result } => {
                self.update = match result {
                    Ok(Some(found)) => UpdateState::Available(found),
                    // A quiet start-up check stays quiet unless there's something to offer
                    _ if !manual => UpdateState::Idle,
                    Ok(None) => UpdateState::UpToDate,
                    Err(err) => UpdateState::Failed(format!("Couldn't check for updates: {err}")),
                };
            }
            Message::InstallUpdate => {
                if let UpdateState::Available(found) = &self.update {
                    let found = found.clone();
                    let progress = Arc::new(Progress::default());
                    self.update = UpdateState::Downloading {
                        version: found.version.clone(),
                        progress: progress.clone(),
                    };
                    let kind = self.install_kind.clone();
                    return Task::perform(
                        async move {
                            update::apply(kind, found, &progress)
                                .await
                                .map_err(|e| format!("{e:#}"))
                        },
                        Message::UpdateApplied,
                    );
                }
            }
            Message::UpdateTick => {}
            Message::UpdateApplied(result) => {
                self.update = match (result, &self.update) {
                    (Ok(()), UpdateState::Downloading { version, .. }) => {
                        UpdateState::Ready(version.clone())
                    }
                    (Ok(()), _) => UpdateState::Idle,
                    (Err(err), _) => UpdateState::Failed(format!("Update failed: {err}")),
                };
            }
            Message::Restart => {
                if self.dirty {
                    self.modal = Some(Modal::Message {
                        title: "Unsaved changes".into(),
                        body: "Save changes before restarting?".into(),
                        buttons: vec![
                            ("Save".into(), Message::RestartSave),
                            ("Discard".into(), Message::RestartNow),
                            ("Cancel".into(), Message::CloseModal),
                        ],
                    });
                } else {
                    return self.restart();
                }
            }
            Message::RestartSave => {
                self.modal = None;
                model::unique_names(&mut self.rules);
                match luaio::write_rules(&self.settings.rules(), &self.rules) {
                    Ok(_) => return self.restart(),
                    Err(err) => self.info("Save failed", err.to_string()),
                }
            }
            Message::RestartNow => {
                self.modal = None;
                return self.restart();
            }
            Message::SettingsCheckUpdates(v) => {
                if let Some(Modal::Settings { check_updates, .. }) = &mut self.modal {
                    *check_updates = v;
                }
            }
        }
        Task::none()
    }

    fn edit(&mut self, f: impl FnOnce(&mut Editor)) {
        if let Some(ed) = &mut self.editor {
            f(ed);
            self.sync();
        }
    }

    fn picker(
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

    fn picker_ok(&mut self) {
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

    fn import_ok(&mut self) -> Task<Message> {
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
        model::unique_names(&mut self.rules);
        if let Err(err) = luaio::write_rules(&self.settings.rules(), &self.rules) {
            self.info("Save failed", err.to_string());
            return Task::none();
        }
        self.dirty = false;
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

    fn settings_dlg(&mut self, f: impl FnOnce(&mut String, &mut String, &mut bool)) {
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

    fn settings_ok(&mut self) -> Task<Message> {
        let Some(Modal::Settings {
            rules,
            hook,
            reload,
            check_updates,
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
                        ("Save here".into(), Message::Save),
                        ("Load file".into(), Message::LoadRules),
                    ],
                });
            } else {
                self.load_rules();
            }
        }
        Task::none()
    }

    // ---- view ---------------------------------------------------------------------------

    fn view(&self) -> Element<'_, Message> {
        let tool = |label: &'static str, msg: Message, tip: &'static str| -> Element<'_, Message> {
            let b = button(text(label).size(14))
                .on_press(msg)
                .style(button::secondary);
            if tip.is_empty() {
                b.into()
            } else {
                tooltip(
                    b,
                    container(text(tip).size(13))
                        .padding(6)
                        .style(container::rounded_box),
                    tooltip::Position::Bottom,
                )
                .into()
            }
        };
        let sep = || space().width(12);
        let tab_button = |label: &'static str, tab: Tab| {
            button(text(label).size(14))
                .on_press(Message::Tab(tab))
                .style(if self.tab == tab {
                    button::primary
                } else {
                    button::text
                })
        };
        let tabs = row![
            tab_button("Window rules", Tab::Rules),
            tab_button("Audio routes", Tab::Audio)
        ]
        .spacing(2);
        let rule_tools: Element<_> = row![
            tool("Add rule", Message::AddRule, "Ctrl+N"),
            tool(
                "From window…",
                Message::FromWindow,
                "Create a rule from an open window (Ctrl+W)"
            ),
            tool("Duplicate", Message::Duplicate, "Ctrl+D"),
            tool("Delete", Message::Delete, "Del"),
            sep(),
            tool(
                "Move up",
                Message::Move(-1),
                "Later rules win when two rules set the same thing (Alt+Up)"
            ),
            tool("Move down", Message::Move(1), "Alt+Down"),
            sep(),
            tool(
                "Import…",
                Message::Import,
                "Import window rules from another Hyprland .lua file"
            ),
            tool("Refresh windows", Message::Refresh, "F5"),
        ]
        .spacing(6)
        .into();
        let tab_tools = match self.tab {
            Tab::Rules => rule_tools,
            Tab::Audio => self.audio.toolbar().map(Message::Audio),
        };
        let toolbar = row![
            tabs,
            sep(),
            tab_tools,
            space().width(Fill),
            tool("Settings", Message::OpenSettings, ""),
            pick_list(
                &ThemePref::ALL[..],
                Some(self.settings.theme),
                Message::Theme
            )
            .text_size(14)
            .width(150),
            button(text("Save").size(14))
                .on_press(Message::Save)
                .style(button::primary),
        ]
        .spacing(6)
        .padding(8);

        let body: Element<_> = match self.tab {
            Tab::Rules => row![
                container(self.view_list()).width(380),
                container(self.view_editor()).width(Fill),
            ]
            .spacing(8)
            .padding([0, 8])
            .height(Fill)
            .into(),
            Tab::Audio => container(self.audio.view().map(Message::Audio))
                .padding([0, 8])
                .height(Fill)
                .into(),
        };

        let hooked = settings::hook_installed(&self.settings.hook(), &self.settings.rules());
        let state = if self.dirty {
            "unsaved changes"
        } else {
            "saved"
        };
        let mut status = format!(
            "{} rule(s) · {} · {state}",
            self.rules.len(),
            self.settings.rules().display()
        );
        if !self.note.is_empty() {
            status = format!("{status} · {}", self.note);
        }
        let mut bar = row![text(status).size(13).width(Fill)]
            .spacing(8)
            .padding(8)
            .align_y(iced::Center);
        bar = bar.push(self.view_update());
        if !hooked {
            bar = bar.push(tooltip(
                button(text("Load rules in Hyprland").size(13))
                    .on_press(Message::InstallHook)
                    .style(button::primary),
                container(
                    text(format!(
                        "Add a line to {} so Hyprland loads {}",
                        self.settings.hook().display(),
                        self.settings.rules().display()
                    ))
                    .size(13),
                )
                .padding(6)
                .style(container::rounded_box),
                tooltip::Position::Top,
            ));
        }

        let base: Element<_> = column![toolbar, body, bar].into();
        match &self.modal {
            None => base,
            Some(m) => overlay(base, self.view_modal(m)),
        }
    }

    fn view_update(&self) -> Element<'_, Message> {
        let small = |s: String| text(s).size(13);
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
                button(text("Update").size(13))
                    .on_press(Message::InstallUpdate)
                    .style(button::primary),
            ]
            .spacing(8)
            .align_y(iced::Center)
            .into(),
            UpdateState::Downloading { version, progress } => row![
                small(format!("Downloading {version} — {}", progress.describe())),
                progress_bar(0.0..=1.0, progress.fraction().unwrap_or(0.0))
                    .length(140)
                    .girth(8),
            ]
            .spacing(8)
            .align_y(iced::Center)
            .into(),
            UpdateState::Ready(version) => row![
                small(format!("Updated to {version}")),
                button(text("Restart").size(13))
                    .on_press(Message::Restart)
                    .style(button::primary),
            ]
            .spacing(8)
            .align_y(iced::Center)
            .into(),
            UpdateState::Failed(err) => row![
                text(err.as_str()).size(13).style(text::danger),
                button(text("Retry").size(13))
                    .on_press(Message::CheckUpdates)
                    .style(button::secondary),
            ]
            .spacing(8)
            .align_y(iced::Center)
            .into(),
        }
    }

    fn view_list(&self) -> Element<'_, Message> {
        let needle = self.search.to_lowercase();
        let mut items = Column::new().spacing(2);
        for (i, r) in self.rules.iter().enumerate() {
            let (title, summary) = (r.title(), r.summary());
            if !needle.is_empty()
                && !format!("{title}\n{summary}")
                    .to_lowercase()
                    .contains(&needle)
            {
                continue;
            }
            let off = if r.enabled { "" } else { "   (off)" };
            let label = column![
                text(format!("{title}{off}")).size(14).font(if r.enabled {
                    Font::DEFAULT
                } else {
                    ITALIC
                }),
                text(summary).size(12).style(text::secondary),
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
                    .on_press(Message::Select(i))
                    .style(style),
            );
        }
        column![
            text_input("Filter rules…", &self.search).on_input(Message::Search),
            scrollable(items.padding(iced::Padding::ZERO.right(12))).height(Fill),
        ]
        .spacing(6)
        .into()
    }

    fn view_editor(&self) -> Element<'_, Message> {
        let (Some(ed), Some(rule)) = (&self.editor, self.selected.and_then(|i| self.rules.get(i)))
        else {
            return center(text("Select a rule, or add one.").style(text::secondary)).into();
        };
        let label = |s: &'static str| text(s).width(110);

        let head = column![
            row![
                label("Name"),
                text_input("optional, e.g. \"Steam games on workspace 2\"", &ed.name)
                    .on_input(Message::Name)
            ]
            .spacing(8)
            .align_y(iced::Center),
            row![
                space().width(110),
                checkbox(ed.enabled)
                    .label("Rule is active")
                    .on_toggle(Message::Enabled)
            ]
            .spacing(8),
        ]
        .spacing(8);

        // Match conditions
        let mut conds = Column::new().spacing(6);
        for (i, m) in rule
            .matches
            .iter()
            .enumerate()
            .filter(|(_, m)| m.is_pattern())
        {
            let invalid = m.mode == Mode::Regex && !model::valid_regex(&m.pattern());
            let placeholder = if m.mode == Mode::Regex {
                "regular expression (full match)"
            } else {
                "plain text — no regex needed"
            };
            let mut r = row![
                pick_list(
                    &KEY_OPTS[..],
                    KEY_OPTS.iter().find(|k| k.0 == m.key).copied(),
                    move |k| Message::MatchKey(i, k)
                )
                .width(140),
                pick_list(
                    &Mode::ALL[..],
                    Some(m.mode),
                    move |mode| Message::MatchMode(i, mode)
                )
                .width(150),
                text_input(placeholder, &m.text)
                    .on_input(move |t| Message::MatchText(i, t))
                    .width(Fill),
            ]
            .spacing(6)
            .align_y(iced::Center);
            if invalid {
                r = r.push(text("⚠ invalid regex").style(text::danger));
            }
            r = r.push(tooltip(
                checkbox(m.ignore_case)
                    .label("Any case")
                    .on_toggle(move |c| Message::MatchCase(i, c)),
                container(text("Ignore upper/lower case differences").size(13))
                    .padding(6)
                    .style(container::rounded_box),
                tooltip::Position::Top,
            ));
            r = r.push(
                button(text("✕"))
                    .on_press(Message::MatchRemove(i))
                    .style(button::text),
            );
            let earlier_same_field = rule.matches[..i]
                .iter()
                .any(|o| o.is_pattern() && o.key == m.key);
            if earlier_same_field {
                conds = conds.push(text("or").size(12).style(text::secondary));
            }
            conds = conds.push(r);
        }
        let preview = self.preview(rule);
        let matches = group(
            "Applies to windows where…",
            column![
                conds,
                row![
                    button(text("+ Add condition").size(14))
                        .on_press(Message::MatchAdd)
                        .style(button::secondary)
                ],
                text("Different fields must all match; several conditions on the same field match if any one does.")
                    .size(12)
                    .style(text::secondary),
                preview
            ]
            .spacing(8)
            .into(),
        );

        // Common effects
        let pair = |a: &str,
                    b: &str,
                    pa: &'static str,
                    pb: &'static str,
                    fa: fn(String) -> Message,
                    fb: fn(String) -> Message| {
            row![
                text_input(pa, a).on_input(fa).width(140),
                text("×"),
                text_input(pb, b).on_input(fb).width(140),
            ]
            .spacing(6)
            .align_y(iced::Center)
        };
        let common = group(
            "Common",
            column![
                row![label("State"), pick_list(&FloatState::ALL[..], Some(ed.state), Message::State).width(260)]
                    .spacing(8)
                    .align_y(iced::Center),
                row![
                    label("Size"),
                    pair(&ed.size_w, &ed.size_h, "width", "height", Message::SizeW, Message::SizeH),
                    button(text("Copy from window…").size(13))
                        .on_press(Message::GeometryFromWindow(Geometry::Size))
                        .style(button::secondary),
                ]
                    .spacing(8)
                    .align_y(iced::Center),
                row![
                    label("Position"),
                    pair(&ed.move_x, &ed.move_y, "x", "y", Message::MoveX, Message::MoveY),
                    button(text("Copy from window…").size(13))
                        .on_press(Message::GeometryFromWindow(Geometry::Position))
                        .style(button::secondary),
                ]
                    .spacing(8)
                    .align_y(iced::Center),
                row![space().width(110), checkbox(ed.center).label("Center on screen").on_toggle(Message::Center)].spacing(8),
                row![
                    label("Workspace"),
                    text_input("e.g. 2, special:music", &ed.workspace).on_input(Message::Workspace).width(Fill),
                    pick_list(&self.workspace_names[..], None::<String>, Message::Workspace).placeholder("Pick…").width(160),
                    checkbox(ed.ws_silent).label("Silent (don't switch to it)").on_toggle(Message::WsSilent),
                ]
                .spacing(8)
                .align_y(iced::Center),
                row![
                    label("Monitor"),
                    text_input("any", &ed.monitor).on_input(Message::Monitor).width(Fill),
                    pick_list(&self.monitor_names[..], None::<String>, Message::Monitor).placeholder("Pick…").width(160),
                ]
                .spacing(8)
                .align_y(iced::Center),
                row![
                    space().width(110),
                    checkbox(ed.pin).label("Pin (show on every workspace, floating only)").on_toggle(Message::Pin)
                ]
                .spacing(8),
                row![space().width(110), checkbox(ed.no_focus).label("Don't focus when it opens").on_toggle(Message::NoFocus)]
                    .spacing(8),
                row![
                    space().width(110),
                    text("Sizes and positions accept pixels (800) or percentages (50%). Positions are relative to the monitor.")
                        .size(13)
                        .style(text::secondary)
                ]
                .spacing(8),
            ]
            .spacing(8)
            .into(),
        );

        // Other properties
        let mut props = Column::new().spacing(6);
        for (i, (k, v)) in ed.props.iter().enumerate() {
            let hint = match effect_kind(k) {
                Some(Kind::Bool) => "true / false",
                Some(Kind::Int) => "number",
                Some(Kind::Float) => "decimal, e.g. 0.5",
                Some(Kind::Pair) => "{ a, b }",
                Some(Kind::Str) | None => "value",
            };
            props = props.push(
                row![
                    text_input("property", k)
                        .on_input(move |s| Message::PropKey(i, s))
                        .width(200),
                    text_input(hint, v)
                        .on_input(move |s| Message::PropValue(i, s))
                        .width(Fill),
                    button(text("✕"))
                        .on_press(Message::PropRemove(i))
                        .style(button::text),
                ]
                .spacing(6)
                .align_y(iced::Center),
            );
        }
        let other = group(
            "Other properties",
            column![
                props,
                row![
                    pick_list(
                        &self.prop_options[..],
                        self.prop_pick.clone(),
                        Message::PropPick
                    )
                    .placeholder("Choose a property…")
                    .width(260),
                    button(text("+ Add property").size(14))
                        .on_press_maybe(self.prop_pick.as_ref().map(|_| Message::PropAdd))
                        .style(button::secondary),
                ]
                .spacing(6),
            ]
            .spacing(8)
            .into(),
        );

        scrollable(
            column![head, matches, common, other]
                .spacing(12)
                .padding(iced::Padding::ZERO.right(16).bottom(16)),
        )
        .height(Fill)
        .into()
    }

    fn preview(&self, rule: &Rule) -> Element<'_, Message> {
        if !rule.has_condition() {
            return text("Add at least one condition — a rule without one matches nothing.")
                .style(text::secondary)
                .into();
        }
        let hits: Vec<&Json> = self
            .clients
            .iter()
            .filter(|c| rule.matches_window(c))
            .collect();
        if hits.is_empty() {
            return text("No open window matches right now.")
                .style(text::secondary)
                .into();
        }
        let mut col = column![text(format!("Matches {} open window(s):", hits.len()))].spacing(2);
        for c in hits.iter().take(8) {
            col = col.push(
                text(format!(
                    "• {} — {}",
                    hypr::str_field(c, "class"),
                    hypr::str_field(c, "title")
                ))
                .size(13),
            );
        }
        if hits.len() > 8 {
            col = col.push(text(format!("…and {} more", hits.len() - 8)).size(13));
        }
        col.into()
    }

    fn view_modal<'a>(&'a self, modal: &'a Modal) -> Element<'a, Message> {
        let ok_cancel = |ok: Option<Message>| {
            row![
                space().width(Fill),
                button(text("Cancel"))
                    .on_press(Message::CloseModal)
                    .style(button::secondary),
                button(text("OK")).on_press_maybe(ok).style(button::primary),
            ]
            .spacing(8)
        };
        match modal {
            Modal::Message { title, body, buttons } => {
                let mut btns = row![space().width(Fill)].spacing(8);
                for (i, (label, msg)) in buttons.iter().enumerate() {
                    let style = if i == 0 { button::primary } else { button::secondary };
                    btns = btns.push(button(text(label.as_str())).on_press(msg.clone()).style(style));
                }
                column![text(title.as_str()).size(18), text(body.as_str()), btns].spacing(16).width(560).into()
            }
            Modal::Picker { clients, selected, use_title, copy_geom, copy_ws, for_current } => {
                let mut list = Column::new().spacing(2);
                for (i, c) in clients.iter().enumerate() {
                    let cells = row![
                        text(hypr::str_field(c, "class")).width(220),
                        text(hypr::str_field(c, "title")).width(Fill),
                        text(hypr::workspace_name(c)).width(110),
                        text(if hypr::floating(c) { "floating" } else { "" }).width(70),
                    ]
                    .spacing(8);
                    let style = if *selected == Some(i) { button::primary } else { button::text };
                    list = list.push(button(cells).width(Fill).on_press(Message::PickerSelect(i)).style(style));
                }
                let mut col = column![
                    text(match for_current {
                        Some(Geometry::Size) => "Copy the size of an open window",
                        Some(Geometry::Position) => "Copy the position of an open window",
                        None => "Create rule from an open window",
                    })
                    .size(18),
                    row![
                        text("Class").width(220),
                        text("Title").width(Fill),
                        text("Workspace").width(110),
                        text("").width(70)
                    ]
                    .spacing(8)
                    .padding([0, 10]),
                    scrollable(list).height(320),
                ]
                .spacing(10)
                .width(860);
                if let Some(part) = for_current {
                    let hint = match part {
                        Geometry::Size => "Its current size replaces the rule's Size.",
                        Geometry::Position => "Its current position (relative to its monitor) replaces the rule's Position.",
                    };
                    return col
                        .push(text(hint).size(13).style(text::secondary))
                        .push(ok_cancel(selected.map(|_| Message::PickerOk)))
                        .into();
                }
                col = col.extend([
                    checkbox(*use_title)
                        .label("Also match the window title (only this exact window, not every window of the app)")
                        .on_toggle(Message::PickerUseTitle)
                        .into(),
                    checkbox(*copy_geom).label("Copy its current size and position").on_toggle(Message::PickerCopyGeom).into(),
                    checkbox(*copy_ws).label("Copy its current workspace").on_toggle(Message::PickerCopyWs).into(),
                    ok_cancel(selected.map(|_| Message::PickerOk)).into(),
                ]);
                col.into()
            }
            Modal::Import { path, rules, checked, can_comment, comment_out, warning } => {
                let mut list = Column::new().spacing(4);
                for (i, r) in rules.iter().enumerate() {
                    list = list.push(
                        checkbox(checked[i])
                            .label(format!("{}   →   {}", r.title(), r.summary()))
                            .on_toggle(move |v| Message::ImportToggle(i, v)),
                    );
                }
                let mut col = column![
                    text(format!("Import rules from {}", path.file_name().unwrap_or_default().to_string_lossy())).size(18),
                    text(format!("Found {} window rule(s) in {}. Choose which to import:", rules.len(), path.display())),
                ]
                .spacing(10)
                .width(800);
                if let Some(w) = warning {
                    col = col.push(text(w.as_str()).style(text::danger));
                }
                col = col.push(scrollable(list).height(300));
                let cb = checkbox(*comment_out)
                    .label("Comment out the imported rules in that file, so they aren't applied twice (a backup is kept)")
                    .on_toggle_maybe(can_comment.then_some(Message::ImportCommentOut));
                col = col.push(cb);
                if !can_comment {
                    col = col.push(
                        text("The rules in this file couldn't be located reliably in its text, so they'll be left in place.")
                            .size(13)
                            .style(text::secondary),
                    );
                }
                col.push(ok_cancel(checked.iter().any(|c| *c).then_some(Message::ImportOk))).into()
            }
            Modal::Settings { rules, hook, reload, check_updates } => column![
                text("Settings").size(18),
                row![
                    text("Rules file").width(110),
                    text_input("", rules).on_input(Message::SettingsRules).width(Fill),
                    button(text("Browse…")).on_press(Message::BrowseRules).style(button::secondary),
                ]
                .spacing(8)
                .align_y(iced::Center),
                row![
                    text("Load it from").width(110),
                    text_input("", hook).on_input(Message::SettingsHook).width(Fill),
                    button(text("Browse…")).on_press(Message::BrowseHook).style(button::secondary),
                ]
                .spacing(8)
                .align_y(iced::Center),
                row![
                    space().width(110),
                    text(
                        "The app adds one line to the “Load it from” file so Hyprland loads the rules file, \
                         and reloads by itself whenever the rules change. Usually ~/.config/hypr/hyprland.lua."
                    )
                    .size(13)
                    .style(text::secondary)
                ]
                .spacing(8),
                row![space().width(110), checkbox(*reload).label("Reload Hyprland after saving").on_toggle(Message::SettingsReload)]
                    .spacing(8),
                row![
                    text("Updates").width(110),
                    text(format!("Version {} · {}", update::current_version(), self.install_kind.describe()))
                        .width(Fill),
                    button(text("Check now"))
                        .on_press_maybe((self.install_kind != InstallKind::Source).then_some(Message::CheckUpdates))
                        .style(button::secondary),
                ]
                .spacing(8)
                .align_y(iced::Center),
                row![
                    space().width(110),
                    checkbox(*check_updates)
                        .label("Check for updates at startup")
                        .on_toggle_maybe((self.install_kind != InstallKind::Source).then_some(Message::SettingsCheckUpdates)),
                ]
                .spacing(8),
                ok_cancel(Some(Message::SettingsOk)),
            ]
            .spacing(12)
            .width(680)
            .into(),
        }
    }
}

const ITALIC: Font = Font {
    style: iced::font::Style::Italic,
    ..Font::DEFAULT
};

fn group<'a>(title: &'a str, content: Element<'a, Message>) -> Element<'a, Message> {
    column![
        text(title).size(15).font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::DEFAULT
        }),
        container(content)
            .padding(12)
            .width(Fill)
            .style(container::bordered_box),
    ]
    .spacing(6)
    .into()
}

fn overlay<'a>(base: Element<'a, Message>, content: Element<'a, Message>) -> Element<'a, Message> {
    let dialog = container(content)
        .padding(20)
        .style(container::bordered_box);
    stack![
        base,
        opaque(center(opaque(dialog)).style(|_| {
            container::Style {
                background: Some(
                    Color {
                        a: 0.55,
                        ..Color::BLACK
                    }
                    .into(),
                ),
                ..Default::default()
            }
        })),
    ]
    .into()
}
