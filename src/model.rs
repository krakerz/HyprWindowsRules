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
pub const PATTERN_KEYS: [&str; 7] = [
    "class",
    "title",
    "initial_class",
    "initial_title",
    "tag",
    "content",
    "xdg_tag",
];
const MATCH_KEYS: [&str; 15] = [
    "class",
    "title",
    "initial_class",
    "initial_title",
    "tag",
    "content",
    "xdg_tag",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Float,
    Str,
    Pair,
}

/// Known effects with the value type the editor offers for them.
pub const EFFECTS: [(&str, Kind); 41] = [
    ("float", Kind::Bool),
    ("tile", Kind::Bool),
    ("size", Kind::Pair),
    ("move", Kind::Pair),
    ("center", Kind::Bool),
    ("workspace", Kind::Str),
    ("monitor", Kind::Str),
    ("pin", Kind::Bool),
    ("fullscreen", Kind::Bool),
    ("maximize", Kind::Bool),
    ("pseudo", Kind::Bool),
    ("no_initial_focus", Kind::Bool),
    ("opacity", Kind::Str),
    ("border_size", Kind::Int),
    ("rounding", Kind::Int),
    ("no_shadow", Kind::Bool),
    ("no_blur", Kind::Bool),
    ("no_anim", Kind::Bool),
    ("no_dim", Kind::Bool),
    ("no_focus", Kind::Bool),
    ("stay_focused", Kind::Bool),
    ("keep_aspect_ratio", Kind::Bool),
    ("persistent_size", Kind::Bool),
    ("no_max_size", Kind::Bool),
    ("scrolling_width", Kind::Float),
    ("idle_inhibit", Kind::Str),
    ("suppress_event", Kind::Str),
    ("render_unfocused", Kind::Bool),
    ("immediate", Kind::Bool),
    ("no_screen_share", Kind::Bool),
    ("decorate", Kind::Bool),
    ("allows_input", Kind::Bool),
    ("opaque", Kind::Bool),
    ("xray", Kind::Bool),
    ("dim_around", Kind::Bool),
    ("fullscreen_state", Kind::Str),
    ("content", Kind::Str),
    ("min_size", Kind::Pair),
    ("max_size", Kind::Pair),
    ("animation", Kind::Str),
    ("tag", Kind::Str),
];

pub fn effect_kind(key: &str) -> Option<Kind> {
    EFFECTS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, kind)| *kind)
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

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub name: String,
    pub enabled: bool,
    pub matches: Vec<MatchField>,
    pub effects: Vec<(String, Value)>,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            matches: Vec::new(),
            effects: Vec::new(),
        }
    }
}

impl Rule {
    pub fn from_spec(spec: Vec<(String, Value)>) -> Self {
        let mut rule = Rule::default();
        let mut rest = Vec::new();
        for (k, v) in spec {
            match k.as_str() {
                "name" => rule.name = v.plain(),
                "enabled" => rule.enabled = v.as_bool() != Some(false),
                "match" => {
                    if let Value::Map(m) = v {
                        rule.matches = ordered(m, &MATCH_KEYS)
                            .into_iter()
                            .map(|(k, v)| MatchField::from_value(&k, v))
                            .collect();
                    }
                }
                _ => rest.push((k, v)),
            }
        }
        rule.effects = ordered(rest, &EFFECT_ORDER);
        rule
    }

    pub fn to_spec(&self) -> Vec<(String, Value)> {
        let mut spec = Vec::new();
        if !self.name.is_empty() {
            spec.push(("name".into(), Value::Str(self.name.clone())));
        }
        if !self.enabled {
            spec.push(("enabled".into(), Value::Bool(false)));
        }
        let m = self
            .matches
            .iter()
            .filter(|m| !m.text.is_empty() || !m.is_pattern() || m.raw.is_some())
            .map(|m| (m.key.clone(), m.spec_value()))
            .collect();
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
        self.matches
            .iter()
            .any(|m| m.is_pattern() && (!m.text.is_empty() || m.raw.is_some()))
    }

    /// Approximate Hyprland's matching against a `hyprctl clients -j` entry.
    pub fn matches_window(&self, client: &Json) -> bool {
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
                    if m.matches(&s) != Some(true) {
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
    let mut seen: std::collections::HashMap<String, u32> = Default::default();
    for r in rules.iter_mut() {
        if r.name.is_empty() {
            continue;
        }
        let count = seen.entry(r.name.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            r.name = format!("{} ({})", r.name, count);
        }
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
}
