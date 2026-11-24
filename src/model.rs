//! Window rule model and the plain-text <-> regex translation behind the match modes.

use serde_json::Value as Json;

/// A Lua value as it appears in a rule spec. Maps keep their order so files round-trip stably.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    Map(Vec<(String, Value)>),
}

impl Value {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn truthy(&self) -> bool {
        !matches!(self, Value::Bool(false))
    }

    /// Plain text shown in summaries and the "Common" widgets.
    pub fn plain(&self) -> String {
        match self {
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Str(s) => s.clone(),
            Value::List(v) => v.iter().map(Value::plain).collect::<Vec<_>>().join(" x "),
            Value::Map(_) => "{…}".into(),
        }
    }
}

/// Hyprland matches rule patterns with RE2::FullMatch, so patterns cover the whole value
/// and ^/$ anchors are redundant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Exact,
    Starts,
    Contains,
    Ends,
    Regex,
}

impl Mode {
    pub const ALL: [Mode; 5] = [
        Mode::Exact,
        Mode::Starts,
        Mode::Contains,
        Mode::Ends,
        Mode::Regex,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Exact => "is exactly",
            Mode::Starts => "starts with",
            Mode::Contains => "contains",
            Mode::Ends => "ends with",
            Mode::Regex => "matches regex",
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Match keys whose value is a pattern; the rest (xwayland, float, ...) take plain values.
pub const PATTERN_KEYS: [&str; 8] = [
    "class",
    "title",
    "initial_class",
    "initial_title",
    "tag",
    "content",
    "xdg_tag",
    "namespace",
];
const MATCH_KEYS: [&str; 16] = [
    "class",
    "title",
    "initial_class",
    "initial_title",
    "tag",
    "content",
    "xdg_tag",
    "namespace",
    "xwayland",
    "float",
    "fullscreen",
    "pin",
    "focus",
    "group",
    "modal",
    "workspace",
];

pub fn match_key_label(key: &str) -> &str {
    match key {
        "class" => "Class",
        "title" => "Title",
        "initial_class" => "Initial class",
        "initial_title" => "Initial title",
        "tag" => "Tag",
        "content" => "Content type",
        "xdg_tag" => "XDG tag",
        "namespace" => "Namespace",
        other => other,
    }
}

/// Display order for effect properties; unknown keys follow alphabetically.
const EFFECT_ORDER: [&str; 17] = [
    "float",
    "tile",
    "size",
    "move",
    "center",
    "workspace",
    "monitor",
    "pin",
    "fullscreen",
    "maximize",
    "no_initial_focus",
    "opacity",
    "border_size",
    "no_shadow",
    "no_blur",
    "no_anim",
    "scrolling_width",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Bool,
    Int,
    Float,
    Str,
    Pair,
}

/// One property a rule can set: the value type the editor offers, a one-line help text
/// and, for enum-like strings, the allowed values (`multi`: several of them, space-separated).
#[derive(Debug, Clone, Copy)]
pub struct PropInfo {
    pub key: &'static str,
    pub kind: Kind,
    pub help: &'static str,
    pub choices: &'static [&'static str],
    pub multi: bool,
}

const fn prop(
    key: &'static str,
    kind: Kind,
    help: &'static str,
    choices: &'static [&'static str],
) -> PropInfo {
    PropInfo {
        key,
        kind,
        help,
        choices,
        multi: false,
    }
}

const fn multi_prop(
    key: &'static str,
    help: &'static str,
    choices: &'static [&'static str],
) -> PropInfo {
    PropInfo {
        multi: true,
        ..prop(key, Kind::Str, help, choices)
    }
}

/// Window-rule effects (Hyprland 0.56 rule enum).
pub const WINDOW_PROPS: [PropInfo; 57] = [
    prop("float", Kind::Bool, "Open the window floating", &[]),
    prop("tile", Kind::Bool, "Force the window to tile", &[]),
    prop(
        "size",
        Kind::Pair,
        "Window size: pixels, or percent of the monitor (50%)",
        &[],
    ),
    prop(
        "move",
        Kind::Pair,
        "Window position: pixels, or percent of the monitor",
        &[],
    ),
    prop(
        "center",
        Kind::Bool,
        "Center the window on its monitor",
        &[],
    ),
    prop(
        "workspace",
        Kind::Str,
        "Open on a workspace: 2, name:web, special:x; add \" silent\" to not switch",
        &[],
    ),
    prop("monitor", Kind::Str, "Open on a monitor (name or id)", &[]),
    prop(
        "pin",
        Kind::Bool,
        "Show the window on every workspace (floating only)",
        &[],
    ),
    prop("fullscreen", Kind::Bool, "Open fullscreen", &[]),
    prop("maximize", Kind::Bool, "Open maximized", &[]),
    prop(
        "pseudo",
        Kind::Bool,
        "Pseudotile: keep the window's own size inside its tile",
        &[],
    ),
    prop(
        "no_initial_focus",
        Kind::Bool,
        "Don't focus the window when it opens",
        &[],
    ),
    prop(
        "opacity",
        Kind::Str,
        "Opacity, e.g. 0.9 or \"0.9 0.7\" (active, inactive)",
        &[],
    ),
    prop("border_size", Kind::Int, "Border width in pixels", &[]),
    prop("rounding", Kind::Int, "Corner radius in pixels", &[]),
    prop("no_shadow", Kind::Bool, "Disable the window shadow", &[]),
    prop("no_blur", Kind::Bool, "Disable blur behind the window", &[]),
    prop(
        "no_anim",
        Kind::Bool,
        "Disable animations for the window",
        &[],
    ),
    prop(
        "no_dim",
        Kind::Bool,
        "Don't dim the window when it is unfocused",
        &[],
    ),
    prop(
        "no_focus",
        Kind::Bool,
        "The window can never take focus",
        &[],
    ),
    prop(
        "stay_focused",
        Kind::Bool,
        "Keep focus on the window while it is visible",
        &[],
    ),
    prop(
        "keep_aspect_ratio",
        Kind::Bool,
        "Keep the aspect ratio while resizing",
        &[],
    ),
    prop(
        "persistent_size",
        Kind::Bool,
        "Remember the size for next time (floating only)",
        &[],
    ),
    prop(
        "no_max_size",
        Kind::Bool,
        "Ignore the maximum size the app asks for",
        &[],
    ),
    prop(
        "scrolling_width",
        Kind::Float,
        "Column width in the scrolling layout (0 to 1)",
        &[],
    ),
    prop(
        "idle_inhibit",
        Kind::Str,
        "When the window prevents the screen from idling",
        &["none", "always", "focus", "fullscreen"],
    ),
    multi_prop(
        "suppress_event",
        "Ignore these app requests",
        &[
            "fullscreen",
            "maximize",
            "activate",
            "activatefocus",
            "fullscreenoutput",
            "x11configurerequest",
        ],
    ),
    prop(
        "render_unfocused",
        Kind::Bool,
        "Keep rendering the window while it isn't visible",
        &[],
    ),
    prop(
        "immediate",
        Kind::Bool,
        "Allow tearing (needs general.allow_tearing)",
        &[],
    ),
    prop(
        "no_screen_share",
        Kind::Bool,
        "Hide the window from screen sharing",
        &[],
    ),
    prop(
        "decorate",
        Kind::Bool,
        "Draw decorations (border, shadow, titlebar)",
        &[],
    ),
    prop(
        "allows_input",
        Kind::Bool,
        "Let the window receive input (for input-transparent surfaces)",
        &[],
    ),
    prop(
        "opaque",
        Kind::Bool,
        "Treat the window as fully opaque",
        &[],
    ),
    prop(
        "xray",
        Kind::Bool,
        "Blur sees through this window to the wallpaper",
        &[],
    ),
    prop(
        "dim_around",
        Kind::Bool,
        "Dim everything around the window (floating only)",
        &[],
    ),
    prop(
        "fullscreen_state",
        Kind::Str,
        "Fullscreen state as \"internal client\", e.g. \"2 0\"",
        &[],
    ),
    prop(
        "content",
        Kind::Str,
        "Content type reported to the compositor",
        &["none", "photo", "video", "game"],
    ),
    prop(
        "min_size",
        Kind::Pair,
        "Smallest size the window can have",
        &[],
    ),
    prop(
        "max_size",
        Kind::Pair,
        "Largest size the window can have",
        &[],
    ),
    prop(
        "animation",
        Kind::Str,
        "Animation style, e.g. slide, popin 80%",
        &[],
    ),
    prop(
        "tag",
        Kind::Str,
        "Add a tag (+name toggles on, name- off) that other rules can match",
        &[],
    ),
    prop(
        "group",
        Kind::Str,
        "Group behaviour: set, new, barred, deny, invade, lock, override, unset",
        &[],
    ),
    prop(
        "no_close_for",
        Kind::Int,
        "Ignore close requests for this many milliseconds after opening",
        &[],
    ),
    prop(
        "rounding_power",
        Kind::Float,
        "Corner curve; 2 is a circle, higher is squarer",
        &[],
    ),
    prop(
        "border_color",
        Kind::Str,
        "Border colour, e.g. rgba(33ccffee)",
        &[],
    ),
    prop(
        "focus_on_activate",
        Kind::Bool,
        "Focus the window when the app asks for attention",
        &[],
    ),
    prop(
        "nearest_neighbor",
        Kind::Bool,
        "Scale with nearest-neighbour instead of smoothing",
        &[],
    ),
    prop(
        "no_follow_mouse",
        Kind::Bool,
        "Hovering over the window doesn't focus it",
        &[],
    ),
    prop(
        "no_shortcuts_inhibit",
        Kind::Bool,
        "Don't let the app grab your shortcuts",
        &[],
    ),
    prop(
        "force_rgbx",
        Kind::Bool,
        "Ignore the alpha channel of the window",
        &[],
    ),
    prop(
        "sync_fullscreen",
        Kind::Bool,
        "Fullscreen state follows the app's own state",
        &[],
    ),
    prop(
        "no_vrr",
        Kind::Bool,
        "Disable variable refresh rate for this window",
        &[],
    ),
    prop(
        "no_auto_hdr",
        Kind::Bool,
        "Don't switch the monitor to HDR for this window",
        &[],
    ),
    prop(
        "tonemap",
        Kind::Str,
        "Tone mapping mode for HDR content",
        &["on", "off", "clamp", "limited"],
    ),
    prop(
        "scroll_mouse",
        Kind::Float,
        "Scroll speed multiplier for a mouse",
        &[],
    ),
    prop(
        "scroll_touchpad",
        Kind::Float,
        "Scroll speed multiplier for a touchpad",
        &[],
    ),
    prop(
        "confine_pointer",
        Kind::Bool,
        "Keep the pointer inside the window",
        &[],
    ),
];

/// `HL.WorkspaceRuleSpec` keys except `enabled`.
pub const WORKSPACE_PROPS: [PropInfo; 17] = [
    prop(
        "workspace",
        Kind::Str,
        "Which workspaces: 2, name:web, special:x, w[tv1], f[1]…",
        &[],
    ),
    prop(
        "monitor",
        Kind::Str,
        "Pin the workspace to a monitor (name or id)",
        &[],
    ),
    prop(
        "default",
        Kind::Bool,
        "Make this the monitor's default workspace",
        &[],
    ),
    prop(
        "default_name",
        Kind::Str,
        "Display name for the workspace",
        &[],
    ),
    prop(
        "persistent",
        Kind::Bool,
        "Keep the workspace alive when empty",
        &[],
    ),
    prop(
        "gaps_in",
        Kind::Str,
        "Gap between windows: pixels, or \"top right bottom left\"",
        &[],
    ),
    prop(
        "gaps_out",
        Kind::Str,
        "Gap to the screen edge: pixels, or \"top right bottom left\"",
        &[],
    ),
    prop(
        "float_gaps",
        Kind::Str,
        "Gap around floating windows: pixels, or \"top right bottom left\"",
        &[],
    ),
    prop("border_size", Kind::Int, "Border width in pixels", &[]),
    prop("no_border", Kind::Bool, "Remove window borders", &[]),
    prop("no_rounding", Kind::Bool, "Remove rounded corners", &[]),
    prop("no_shadow", Kind::Bool, "Remove window shadows", &[]),
    prop("decorate", Kind::Bool, "Draw window decorations", &[]),
    prop(
        "layout",
        Kind::Str,
        "Layout for this workspace: dwindle, master, scrolling…",
        &[],
    ),
    prop(
        "layout_opts",
        Kind::Str,
        "Layout options (a table; kept as loaded)",
        &[],
    ),
    prop(
        "animation",
        Kind::Str,
        "Workspace animation style, e.g. slide, fade",
        &[],
    ),
    prop(
        "on_created_empty",
        Kind::Str,
        "Command to run when the workspace is created empty",
        &[],
    ),
];

/// `HL.LayerRuleSpec` keys except `enabled`, `match` and `name`.
pub const LAYER_PROPS: [PropInfo; 10] = [
    prop(
        "above_lock",
        Kind::Int,
        "Draw above the lock screen (1 above, 2 also receives input)",
        &[],
    ),
    prop(
        "animation",
        Kind::Str,
        "Animation style for the layer, e.g. slide, popin, fade",
        &[],
    ),
    prop("blur", Kind::Bool, "Blur behind the layer", &[]),
    prop(
        "blur_popups",
        Kind::Bool,
        "Blur behind the layer's popups too",
        &[],
    ),
    prop(
        "dim_around",
        Kind::Bool,
        "Dim everything around the layer",
        &[],
    ),
    prop(
        "ignore_alpha",
        Kind::Float,
        "Don't blur pixels below this alpha (0 to 1)",
        &[],
    ),
    prop(
        "no_anim",
        Kind::Bool,
        "Disable animations for the layer",
        &[],
    ),
    prop(
        "no_screen_share",
        Kind::Bool,
        "Hide the layer from screen sharing",
        &[],
    ),
    prop(
        "order",
        Kind::Int,
        "Stacking order inside its layer (higher is on top)",
        &[],
    ),
    prop(
        "xray",
        Kind::Bool,
        "Blur sees through this layer to the wallpaper",
        &[],
    ),
];

pub fn props_for(kind: RuleKind) -> &'static [PropInfo] {
    match kind {
        RuleKind::Window => &WINDOW_PROPS,
        RuleKind::Workspace => &WORKSPACE_PROPS,
        RuleKind::Layer => &LAYER_PROPS,
    }
}

const RE_META: &str = r".^$*+?()[]{}|\";

fn is_meta(c: char) -> bool {
    RE_META.contains(c)
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if is_meta(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub fn build_pattern(mode: Mode, text: &str, ignore_case: bool) -> String {
    let body = if mode == Mode::Regex {
        text.to_string()
    } else {
        let lit = escape(text);
        match mode {
            Mode::Exact => lit,
            Mode::Starts => lit + ".*",
            Mode::Contains => format!(".*{lit}.*"),
            Mode::Ends => format!(".*{lit}"),
            Mode::Regex => unreachable!(),
        }
    };
    if ignore_case {
        format!("(?i){body}")
    } else {
        body
    }
}

/// The literal text `s` stands for, or None if it uses regex features.
///
/// A bare "." is accepted as a literal dot: configs often write class = "org.app.Name"
/// unescaped and mean the literal name.
fn unescape_literal(s: &str) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            if i + 1 < chars.len() && is_meta(chars[i + 1]) {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            return None;
        }
        if c == '.' && !(i + 1 < chars.len() && "*+?{".contains(chars[i + 1])) {
            out.push(c);
        } else if is_meta(c) {
            return None;
        } else {
            out.push(c);
        }
        i += 1;
    }
    Some(out)
}

/// '^(x)$' / '^x$' -> 'x' (anchors are implied by full matching).
fn strip_anchor_group(p: &str) -> &str {
    if p.len() >= 2 && p.starts_with('^') && p.ends_with('$') && !p.ends_with("\\$") {
        let inner = &p[1..p.len() - 1];
        if inner.len() >= 2 && inner.starts_with('(') && inner.ends_with(')') && single_group(inner)
        {
            return &inner[1..inner.len() - 1];
        }
        return inner;
    }
    p
}

/// True if s = '( ... )' is one group spanning the whole string with no top-level '|'.
fn single_group(s: &str) -> bool {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 && i != b.len() - 1 {
                    return false;
                }
            }
            b'|' if depth == 1 => return false,
            _ => {}
        }
        i += 1;
    }
    depth == 0
}

/// Split a pattern into (mode, text, ignore_case), falling back to (Regex, pattern, ...).
pub fn infer_mode(pattern: &str) -> (Mode, String, bool) {
    let ignore_case = pattern.starts_with("(?i)");
    let p = if ignore_case { &pattern[4..] } else { pattern };
    let core = strip_anchor_group(p);
    for (mode, prefix, suffix) in [
        (Mode::Contains, ".*", ".*"),
        (Mode::Starts, "", ".*"),
        (Mode::Ends, ".*", ""),
        (Mode::Exact, "", ""),
    ] {
        if core.starts_with(prefix)
            && core.ends_with(suffix)
            && core.len() >= prefix.len() + suffix.len()
            && let Some(lit) = unescape_literal(&core[prefix.len()..core.len() - suffix.len()])
            && !lit.is_empty()
        {
            return (mode, lit, ignore_case);
        }
    }
    (Mode::Regex, p.to_string(), ignore_case)
}

/// Hyprland's patterns are RE2; the `regex` crate shares its syntax (no lookaround/backrefs).
pub fn valid_regex(p: &str) -> bool {
    regex::Regex::new(p).is_ok()
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchField {
    pub key: String,
    pub mode: Mode,
    pub text: String,
    pub ignore_case: bool,
    /// For non-pattern keys (bool/number).
    pub value: Option<Value>,
    /// Original pattern; kept verbatim until the field is edited.
    pub raw: Option<String>,
}

impl MatchField {
    pub fn new(key: &str) -> Self {
        Self {
            key: key.into(),
            mode: Mode::Exact,
            text: String::new(),
            ignore_case: false,
            value: None,
            raw: None,
        }
    }

    pub fn exact(key: &str, text: &str) -> Self {
        Self {
            text: text.into(),
            ..Self::new(key)
        }
    }

    pub fn from_value(key: &str, value: Value) -> Self {
        match value {
            Value::Str(s) if PATTERN_KEYS.contains(&key) => {
                let (mode, text, ignore_case) = infer_mode(&s);
                Self {
                    key: key.into(),
                    mode,
                    text,
                    ignore_case,
                    value: None,
                    raw: Some(s),
                }
            }
            v => Self {
                value: Some(v),
                ..Self::new(key)
            },
        }
    }

    pub fn is_pattern(&self) -> bool {
        self.value.is_none()
    }

    pub fn pattern(&self) -> String {
        match &self.raw {
            Some(raw) => raw.clone(),
            None => build_pattern(self.mode, &self.text, self.ignore_case),
        }
    }

    fn spec_value(&self) -> Value {
        match &self.value {
            Some(v) => v.clone(),
            None => Value::Str(self.pattern()),
        }
    }

    /// Full-match against `actual`; None if the pattern doesn't compile.
    pub fn matches(&self, actual: &str) -> Option<bool> {
        let re = regex::Regex::new(&format!("^(?:{})$", self.pattern())).ok()?;
        Some(re.is_match(actual))
    }

    pub fn describe(&self) -> String {
        if let Some(v) = &self.value {
            return format!("{} = {}", self.key, v.plain());
        }
        if let Some(raw) = &self.raw
            && infer_mode(raw).0 == Mode::Regex
        {
            return format!("{} ~ {}", self.key, raw);
        }
        format!(
            "{} {} \"{}\"{}",
            self.key,
            self.mode.label(),
            self.text,
            if self.ignore_case { " (any case)" } else { "" }
        )
    }
}

/// Which Hyprland call a rule is written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RuleKind {
    #[default]
    Window,
    Workspace,
    Layer,
}

impl RuleKind {
    pub const ALL: [RuleKind; 3] = [RuleKind::Window, RuleKind::Workspace, RuleKind::Layer];

    pub fn label(self) -> &'static str {
        match self {
            RuleKind::Window => "Windows",
            RuleKind::Workspace => "Workspaces",
            RuleKind::Layer => "Layers",
        }
    }

    /// Effects in the order a file read puts them (workspace selector first, then A-Z).
    fn order_effects(self, effects: Vec<(String, Value)>) -> Vec<(String, Value)> {
        match self {
            RuleKind::Window | RuleKind::Layer => ordered(effects, &EFFECT_ORDER),
            RuleKind::Workspace => ordered(effects, &["workspace"]),
        }
    }

    pub fn lua_fn(self) -> &'static str {
        match self {
            RuleKind::Window => "window_rule",
            RuleKind::Workspace => "workspace_rule",
            RuleKind::Layer => "layer_rule",
        }
    }
}

impl std::fmt::Display for RuleKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A named set of rules that is collapsed and switched on or off together.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub name: String,
    pub enabled: bool,
    pub matches: Vec<MatchField>,
    pub effects: Vec<(String, Value)>,
    pub kind: RuleKind,
    /// Group name; "" = ungrouped. Lives in comment markers, never in the spec.
    pub group: String,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            matches: Vec::new(),
            effects: Vec::new(),
            kind: RuleKind::Window,
            group: String::new(),
        }
    }
}

impl Rule {
    pub fn from_spec(kind: RuleKind, spec: Vec<(String, Value)>) -> Self {
        let mut rule = Rule {
            kind,
            ..Rule::default()
        };
        let mut rest = Vec::new();
        for (k, v) in spec {
            match k.as_str() {
                "name" => rule.name = v.plain(),
                "enabled" => rule.enabled = v.as_bool() != Some(false),
                "match" => {
                    if let Value::Map(m) = v {
                        rule.matches = ordered(m, &MATCH_KEYS)
                            .into_iter()
                            .flat_map(|(k, v)| match v {
                                // One field holding "(?:a)|(?:b)" is how OR'ed rows are saved
                                Value::Str(p) if PATTERN_KEYS.contains(&k.as_str()) => {
                                    split_alternatives(&p)
                                        .into_iter()
                                        .map(|alt| MatchField::from_value(&k, Value::Str(alt)))
                                        .collect::<Vec<_>>()
                                }
                                v => vec![MatchField::from_value(&k, v)],
                            })
                            .collect();
                    }
                }
                _ => rest.push((k, v)),
            }
        }
        rule.effects = rule.kind.order_effects(rest);
        rule
    }

    pub fn to_spec(&self) -> Vec<(String, Value)> {
        let mut spec = Vec::new();
        if self.kind == RuleKind::Workspace {
            // The selector comes first; WorkspaceRuleSpec has no name or match.
            if !self.enabled {
                spec.push(("enabled".into(), Value::Bool(false)));
            }
            spec.extend(ordered(self.effects.clone(), &["workspace"]));
            return spec;
        }
        if !self.name.is_empty() {
            spec.push(("name".into(), Value::Str(self.name.clone())));
        }
        if !self.enabled {
            spec.push(("enabled".into(), Value::Bool(false)));
        }
        // Hyprland's match table holds one value per field, so several rows on the
        // same field are OR'ed into a single pattern.
        let mut m: Vec<(String, Value)> = Vec::new();
        let mut alternatives: Vec<(String, Vec<String>)> = Vec::new();
        for f in self
            .matches
            .iter()
            .filter(|m| !m.text.is_empty() || !m.is_pattern() || m.raw.is_some())
        {
            if f.is_pattern() {
                match alternatives.iter_mut().find(|(k, _)| *k == f.key) {
                    Some((_, alts)) => alts.push(f.pattern()),
                    None => {
                        alternatives.push((f.key.clone(), vec![f.pattern()]));
                        m.push((f.key.clone(), Value::Str(String::new())));
                    }
                }
            } else {
                m.push((f.key.clone(), f.spec_value()));
            }
        }
        for (key, alts) in alternatives {
            let joined = if alts.len() == 1 {
                alts.into_iter().next().unwrap()
            } else {
                alts.iter()
                    .map(|a| format!("(?:{a})"))
                    .collect::<Vec<_>>()
                    .join("|")
            };
            if let Some(slot) = m.iter_mut().find(|(k, _)| *k == key) {
                slot.1 = Value::Str(joined);
            }
        }
        spec.push(("match".into(), Value::Map(m)));
        spec.extend(self.effects.iter().cloned());
        spec
    }

    pub fn effect(&self, key: &str) -> Option<&Value> {
        self.effects.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn title(&self) -> String {
        if !self.name.is_empty() {
            self.name.clone()
        } else if self.kind == RuleKind::Workspace {
            match self.effect("workspace").map(Value::plain) {
                Some(sel) if !sel.is_empty() => format!("Workspace {sel}"),
                _ => "(new rule)".into(),
            }
        } else if !self.matches.is_empty() {
            self.matches
                .iter()
                .map(MatchField::describe)
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            "(new rule)".into()
        }
    }

    pub fn summary(&self) -> String {
        let parts: Vec<String> = self
            .effects
            .iter()
            .map(|(k, v)| match v {
                Value::Bool(true) => k.clone(),
                Value::Bool(false) => format!("not {k}"),
                v => format!("{k} {}", v.plain()),
            })
            .collect();
        if parts.is_empty() {
            "no effects".into()
        } else {
            parts.join(", ")
        }
    }

    pub fn has_condition(&self) -> bool {
        if self.kind == RuleKind::Workspace {
            return self
                .effect("workspace")
                .is_some_and(|v| !v.plain().is_empty());
        }
        self.matches
            .iter()
            .any(|m| m.is_pattern() && (!m.text.is_empty() || m.raw.is_some()))
    }

    /// Approximate Hyprland's matching against a `hyprctl clients -j` entry.
    pub fn matches_window(&self, client: &Json) -> bool {
        if self.kind != RuleKind::Window {
            return false;
        }
        if !self.matches.iter().any(MatchField::is_pattern) {
            return false;
        }
        for m in &self.matches {
            let Some(actual) = client_value(client, &m.key) else {
                continue;
            };
            match &m.value {
                None => {
                    let s = match actual {
                        Json::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    // Rows on the same field are alternatives: one of them must match
                    let any = self
                        .matches
                        .iter()
                        .filter(|o| o.key == m.key && o.is_pattern())
                        .any(|o| o.matches(&s) == Some(true));
                    if !any {
                        return false;
                    }
                }
                Some(v) => {
                    if json_truthy(actual) != v.truthy() {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// "(?:a)|(?:b)" → ["a", "b"]; anything else stays one pattern.
fn split_alternatives(p: &str) -> Vec<String> {
    let b = p.as_bytes();
    let mut parts = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !p[i..].starts_with("(?:") {
            return vec![p.to_string()];
        }
        let (mut depth, mut j) = (0i32, i);
        while j < b.len() {
            match b[j] {
                b'\\' => j += 1,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        if j >= b.len() {
            return vec![p.to_string()];
        }
        parts.push(p[i + 3..j].to_string());
        i = j + 1;
        if i < b.len() {
            if b[i] != b'|' {
                return vec![p.to_string()];
            }
            i += 1;
        }
    }
    if parts.len() < 2 {
        vec![p.to_string()]
    } else {
        parts
    }
}

fn json_truthy(v: &Json) -> bool {
    match v {
        Json::Bool(b) => *b,
        Json::Number(n) => n.as_f64() != Some(0.0),
        Json::Null => false,
        Json::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn client_value<'a>(client: &'a Json, key: &str) -> Option<&'a Json> {
    let field = match key {
        "class" => "class",
        "title" => "title",
        "initial_class" => "initialClass",
        "initial_title" => "initialTitle",
        "xwayland" => "xwayland",
        "float" => "floating",
        "pin" => "pinned",
        "fullscreen" => "fullscreen",
        _ => return None,
    };
    client.get(field).filter(|v| !v.is_null())
}

fn ordered(mut items: Vec<(String, Value)>, order: &[&str]) -> Vec<(String, Value)> {
    let mut out = Vec::with_capacity(items.len());
    for key in order {
        if let Some(i) = items.iter().position(|(k, _)| k == key) {
            out.push(items.remove(i));
        }
    }
    items.sort_by(|a, b| a.0.cmp(&b.0));
    out.extend(items);
    out
}

/// Make non-empty rule names unique so each rule stays identifiable in Hyprland.
pub fn unique_names(rules: &mut [Rule]) {
    use std::collections::HashSet;
    let original: HashSet<String> = rules.iter().map(|r| r.name.clone()).collect();
    let mut taken: HashSet<String> = HashSet::new();
    for r in rules.iter_mut() {
        if r.name.is_empty() || taken.insert(r.name.clone()) {
            continue;
        }
        // Skip suffixes another rule already uses (one really called "x (2)")
        let name = (2..)
            .map(|n| format!("{} ({n})", r.name))
            .find(|n| !taken.contains(n) && !original.contains(n))
            .unwrap();
        taken.insert(name.clone());
        r.name = name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_modes() {
        assert_eq!(
            infer_mode("^(steam)$"),
            (Mode::Exact, "steam".into(), false)
        );
        assert_eq!(
            infer_mode("^(steam_app.*)$"),
            (Mode::Starts, "steam_app".into(), false)
        );
        assert_eq!(
            infer_mode("org.mozilla.Thunderbird"),
            (Mode::Exact, "org.mozilla.Thunderbird".into(), false)
        );
        assert_eq!(infer_mode("(?i)^(teams-for-linux|teams)$").0, Mode::Regex);
        assert_eq!(infer_mode(".*foo.*"), (Mode::Contains, "foo".into(), false));
    }

    #[test]
    fn full_match_preview() {
        let f = MatchField::from_value("class", Value::Str("^(opera)$".into()));
        assert_eq!(f.matches("Opera"), Some(false));
        assert_eq!(f.matches("opera"), Some(true));
        let f = MatchField::from_value("class", Value::Str("a|b".into()));
        assert_eq!(f.matches("ab"), Some(false));
    }

    #[test]
    fn same_field_rows_are_ored() {
        let mut rule = Rule {
            matches: vec![
                MatchField::exact("class", "org.kde.dolphin"),
                MatchField {
                    mode: Mode::Starts,
                    ..MatchField::exact("title", "Extracting")
                },
                MatchField {
                    mode: Mode::Starts,
                    ..MatchField::exact("title", "Copy")
                },
            ],
            ..Default::default()
        };
        let spec = rule.to_spec();
        let Value::Map(m) = &spec.iter().find(|(k, _)| k == "match").unwrap().1 else {
            panic!()
        };
        assert_eq!(
            m[1],
            (
                "title".into(),
                Value::Str("(?:Extracting.*)|(?:Copy.*)".into())
            )
        );
        let back = Rule::from_spec(RuleKind::Window, spec);
        assert_eq!(back.matches.len(), 3);
        assert_eq!(
            (back.matches[2].mode, back.matches[2].text.as_str()),
            (Mode::Starts, "Copy")
        );
        rule = back;
        let win = |t: &str| serde_json::json!({"class": "org.kde.dolphin", "title": t});
        assert!(rule.matches_window(&win("Copying - Dolphin")));
        assert!(rule.matches_window(&win("Extracting x")));
        assert!(!rule.matches_window(&win("Home - Dolphin")));
        assert_eq!(
            split_alternatives("(?:a(b)c)|(?:(?i)d)"),
            vec!["a(b)c", "(?i)d"]
        );
        assert_eq!(split_alternatives("a|b"), vec!["a|b"]);
    }

    #[test]
    fn names_end_up_unique() {
        let named = |n: &str| Rule {
            name: n.into(),
            ..Default::default()
        };
        let mut rules = vec![named("x"), named("x"), named("x (2)"), named(""), named("")];
        unique_names(&mut rules);
        let names: Vec<_> = rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["x", "x (3)", "x (2)", "", ""]);
    }

    #[test]
    fn prop_tables_are_documented() {
        for kind in RuleKind::ALL {
            assert!(props_for(kind).iter().all(|p| !p.help.is_empty()));
        }
        let find = |kind, key| props_for(kind).iter().find(|p| p.key == key);
        assert_eq!(
            find(RuleKind::Window, "idle_inhibit")
                .unwrap()
                .choices
                .len(),
            4
        );
        assert!(find(RuleKind::Window, "suppress_event").unwrap().multi);
        assert!(find(RuleKind::Layer, "float").is_none());
    }

    #[test]
    fn workspace_spec_round_trips() {
        let spec = vec![
            ("workspace".to_string(), Value::Str("w[tv1]".into())),
            ("gaps_out".to_string(), Value::Int(0)),
        ];
        let rule = Rule::from_spec(RuleKind::Workspace, spec.clone());
        assert_eq!(rule.kind, RuleKind::Workspace);
        assert_eq!(rule.effects, spec);
        assert!(rule.matches.is_empty());
        assert_eq!(rule.to_spec(), spec);
        assert_eq!(rule.title(), "Workspace w[tv1]");
        assert!(rule.has_condition());
        assert!(!Rule::from_spec(RuleKind::Workspace, vec![]).has_condition());
    }

    #[test]
    fn layer_spec_round_trips() {
        let spec = vec![
            (
                "match".to_string(),
                Value::Map(vec![("namespace".into(), Value::Str("^(waybar)$".into()))]),
            ),
            ("blur".to_string(), Value::Bool(true)),
        ];
        let rule = Rule::from_spec(RuleKind::Layer, spec.clone());
        assert_eq!(rule.matches[0].key, "namespace");
        assert!(rule.has_condition());
        assert_eq!(rule.to_spec(), spec);
        assert!(!rule.matches_window(&serde_json::json!({"namespace": "waybar"})));
    }
}
