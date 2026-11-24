//! iced user interface.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use iced::keyboard::{self, Key, key::Named};
use iced::{Subscription, Task, Theme, window};
use serde_json::Value as Json;

use crate::audio_ui::{self, AudioTab};
use crate::model::{
    self, Kind, MatchField, Mode, PATTERN_KEYS, Rule, RuleKind, Value, effect_kind,
};
use crate::settings::{self, Settings, ThemePref};
use crate::update::{self, InstallKind, Progress, Update};
use crate::{hypr, luaio};

mod dialogs;
mod editor;
mod history;
mod inspect;
mod list;
pub(crate) mod style;
mod undo;
mod view;

use editor::{Editor, FloatState, KeyOpt};

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

/// Whether `save` runs the pre-save checks.
enum Check {
    Validate,
    Skip,
}

const APP_TITLE: &str = "Hyprland Windows Rules";

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
        // Theme active when the dialog opened; Cancel restores it (the pick applies live).
        theme: ThemePref,
    },
    Inspector,
    History,
}

pub struct App {
    settings: Settings,
    rules: Vec<Rule>,
    groups: Vec<model::Group>,
    /// The rules as last loaded or saved, to mark modified rows.
    saved_rules: Vec<Rule>,
    clients_gen: u64,
    list: list::State,
    inspect: inspect::State,
    history: history::State,
    undo: undo::Stack,
    selected: Option<usize>,
    editor: Option<Editor>,
    dirty: bool,
    clients: Vec<Json>,
    monitors: Vec<Json>,
    monitor_names: Vec<String>,
    live_workspaces: Vec<String>,
    /// Existing workspaces plus any the rules mention, for the workspace drop-down.
    workspace_names: Vec<String>,
    prop_pick: Option<String>,
    /// Options of the editor's Group combo box (the other rules' groups).
    group_combo: iced::widget::combo_box::State<String>,
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
    /// The rules tab's save, from a dialog button (Save is a shortcut, ignored under dialogs).
    SaveRules,
    LoadRules,
    InstallHook,
    InstallHookConfirmed,
    Reloaded(Result<Vec<String>, String>),
    // list
    Search(String),
    Select(usize),
    List(list::Msg),
    Inspect(inspect::Msg),
    History(history::Msg),
    Group(String),
    Undo,
    Redo,
    FocusFilter,
    SaveUnchecked,
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
        Key::Character("f") if modifiers.command() => Some(Message::FocusFilter),
        // Shift makes the character arrive upper-case on some layouts
        Key::Character("z" | "Z") if modifiers.command() && modifiers.shift() => {
            Some(Message::Redo)
        }
        Key::Character("z") if modifiers.command() => Some(Message::Undo),
        Key::Character("y") if modifiers.command() => Some(Message::Redo),
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
        let mut app = App {
            settings: Settings::load(),
            rules: Vec::new(),
            groups: Vec::new(),
            saved_rules: Vec::new(),
            clients_gen: 0,
            list: Default::default(),
            inspect: Default::default(),
            history: Default::default(),
            undo: Default::default(),
            selected: None,
            editor: None,
            dirty: false,
            clients: Vec::new(),
            monitors: Vec::new(),
            monitor_names: Vec::new(),
            live_workspaces: Vec::new(),
            workspace_names: Vec::new(),
            prop_pick: None,
            group_combo: iced::widget::combo_box::State::new(Vec::new()),
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
        self.groups.clear();
        if path.exists() {
            match luaio::read_rules(&path) {
                Ok(res) => {
                    self.rules = res.rules;
                    self.groups = res.groups;
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
        self.saved_rules = self.rules.clone();
        self.undo.reset();
        self.select(if self.rules.is_empty() { None } else { Some(0) });
    }

    /// Write the rules (and groups) to `path`, making rule names unique first.
    fn write_file(&mut self, path: &Path) -> std::io::Result<Option<PathBuf>> {
        model::unique_names(&mut self.rules);
        luaio::write_rules_grouped(path, &self.rules, &self.groups)
    }

    pub(super) fn is_rule_modified(&self, i: usize) -> bool {
        self.rules
            .get(i)
            .is_some_and(|r| !self.saved_rules.contains(r))
    }

    /// Keep `groups` in step with the rules' group names: add missing, drop unused.
    fn ensure_groups(&mut self) {
        for r in &self.rules {
            if !r.group.is_empty() && !self.groups.iter().any(|g| g.name == r.group) {
                self.groups.push(model::Group {
                    name: r.group.clone(),
                    enabled: true,
                });
            }
        }
        let rules = &self.rules;
        self.groups
            .retain(|g| rules.iter().any(|r| r.group == g.name));
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
        self.clients_gen += 1;
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
        let group = ed.group.trim().to_string();
        if let Some(rule) = self.current() {
            rule.name = name;
            rule.enabled = enabled;
            rule.effects = effects;
            rule.group = group;
        }
        self.ensure_groups();
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
        self.save_inner(reload, Check::Validate)
    }

    fn save_inner(&mut self, reload: bool, check: Check) -> Result<Task<Message>, ()> {
        if matches!(check, Check::Validate) {
            let problems = luaio::validate(&self.rules, &self.groups);
            if !problems.is_empty() {
                self.show_problems(&problems);
                return Ok(Task::none());
            }
        }
        if let Err(err) = self.write_file(&self.settings.rules()) {
            self.info("Save failed", err.to_string());
            return Err(());
        }
        self.dirty = false;
        self.saved_rules = self.rules.clone();
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

    fn show_problems(&mut self, problems: &[luaio::Problem]) {
        const MAX: usize = 12;
        let mut body: Vec<String> = problems
            .iter()
            .take(MAX)
            .map(|p| format!("• {}", p.message))
            .collect();
        if problems.len() > MAX {
            body.push(format!("…and {} more", problems.len() - MAX));
        }
        self.modal = Some(Modal::Message {
            title: "Check before saving".into(),
            body: body.join("\n"),
            buttons: vec![
                ("Save anyway".into(), Message::SaveUnchecked),
                ("Cancel".into(), Message::CloseModal),
            ],
        });
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
        let res = match luaio::evaluate_rules(path, Some(&self.settings.rules())) {
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
        let located = std::fs::read_to_string(path).is_ok_and(|s| {
            let spans = luaio::find_window_rule_calls(&s);
            spans.len() == res.rules.len() && luaio::calls_on_own_lines(&s, &spans)
        });
        let can_comment = located && res.error.is_none();
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
        let key = undo::key(&message);
        let before = self.snapshot();
        let task = self.handle(message);
        self.record(before, key);
        self.refresh_analysis();
        self.sync_group_combo();
        task
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
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
                    | Message::Undo
                    | Message::Redo
                    | Message::FocusFilter
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
                    | Message::Undo
                    | Message::Redo
            )
        {
            return Task::none();
        }
        match message {
            Message::Tab(tab) => self.tab = tab,
            Message::Audio(msg) => return self.audio.update(msg).map(Message::Audio),
            Message::AddRule => {
                let kind = self.list.kind;
                let rule = match kind {
                    RuleKind::Window => Rule {
                        matches: vec![MatchField::new("class")],
                        ..Default::default()
                    },
                    RuleKind::Layer => Rule {
                        kind,
                        matches: vec![MatchField::new("namespace")],
                        ..Default::default()
                    },
                    RuleKind::Workspace => Rule {
                        kind,
                        effects: vec![("workspace".into(), Value::Str(String::new()))],
                        ..Default::default()
                    },
                };
                self.insert(rule);
            }
            Message::FromWindow => {
                self.list.kind = RuleKind::Window;
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
                    // Swap with the nearest rule of the same kind in that direction
                    let kind = self.rules[i].kind;
                    let step = delta.signum();
                    let mut j = i as isize + step;
                    while j >= 0 && (j as usize) < self.rules.len() {
                        if self.rules[j as usize].kind == kind {
                            self.rules.swap(i, j as usize);
                            self.dirty = true;
                            self.selected = Some(j as usize);
                            break;
                        }
                        j += step;
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
                    theme: self.settings.theme,
                });
            }
            Message::Save if self.tab == Tab::Audio => {
                self.modal = None;
                return self.audio.save().map(Message::Audio);
            }
            Message::Save | Message::SaveRules => {
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
                    && let Err(err) = self.write_file(&rules)
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
            Message::List(m) => return self.list_update(m),
            Message::Inspect(m) => return self.inspect_update(m),
            Message::History(m) => return self.history_update(m),
            Message::Group(v) => self.edit(|e| e.group = v),
            Message::Undo => self.undo_step(),
            Message::Redo => self.redo_step(),
            Message::FocusFilter => return iced::widget::operation::focus(list::FILTER_ID),
            Message::SaveUnchecked => {
                self.modal = None;
                let reload = self.settings.reload_on_save;
                return self
                    .save_inner(reload, Check::Skip)
                    .unwrap_or_else(|_| Task::none());
            }

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
                    let key = if rule.kind == RuleKind::Layer {
                        &"namespace"
                    } else {
                        PATTERN_KEYS
                            .iter()
                            .find(|k| !rule.matches.iter().any(|m| m.key == **k))
                            .unwrap_or(&"title")
                    };
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

            Message::CloseModal => {
                self.list.menu = None;
                if let Some(Modal::Settings { theme, .. }) = self.modal.take()
                    && theme != self.settings.theme
                {
                    self.settings.theme = theme;
                    if let Err(err) = self.settings.save() {
                        self.info("Settings", format!("Couldn't save settings:\n\n{err}"));
                    }
                }
            }
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
                match self.write_file(&self.settings.rules()) {
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
                match self.write_file(&self.settings.rules()) {
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
}
