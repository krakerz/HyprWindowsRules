//! Read window rules out of Hyprland Lua config files, and write them back.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use mlua::{HookTriggers, Lua, VmState};

use crate::model::{self, Group, Rule, RuleKind, Value};

// Runs the target file with a fake `hl` that records hl.window_rule(), hl.workspace_rule() and hl.layer_rule() calls and turns every
// other hl.* access into a harmless no-op, so arbitrary Hyprland configs evaluate cleanly.
const STUB: &str = r#"
local target, dir, skipped = ...
local rules = {}

local function dummy()
  local d
  d = setmetatable({}, {
    __index = function() return d end,
    __call = function() return d end,
    __concat = function() return "" end,
    __tostring = function() return "" end,
  })
  return d
end

hl = setmetatable({}, { __index = function() return dummy() end })
local function recorder(kind)
  return function(spec)
    rules[#rules + 1] = { kind = kind, spec = spec }
    return dummy()
  end
end
hl.window_rule = recorder("window")
hl.workspace_rule = recorder("workspace")
hl.layer_rule = recorder("layer")

-- Config files must not be able to run things (or exit this process) while being imported
os.execute = function() return nil end
os.exit = function() error("os.exit called") end
os.remove = function() return nil end
os.rename = function() return nil end
io.popen = nil
package.path = dir .. "/?.lua;" .. package.path
-- The managed rules file is skipped: a config with the hook would otherwise list its rules again
local real_require = require
require = function(name)
  local file = package.searchpath(name, package.path)
  if file and skipped(file) then return dummy() end
  local ok, mod = pcall(real_require, name)
  if ok then return mod end
  return dummy()
end
local real_dofile = dofile
dofile = function(file, ...)
  if file and skipped(file) then return dummy() end
  return real_dofile(file, ...)
end

local chunk, err = loadfile(target)
if not chunk then return rules, err end
local ok, runerr = pcall(chunk)
return rules, (not ok) and tostring(runerr) or nil
"#;

const TIMEOUT: Duration = Duration::from_secs(5);

pub struct ImportResult {
    pub rules: Vec<Rule>,
    /// Runtime error part-way through the file; rules before it still count.
    pub error: Option<String>,
}

/// Evaluate `path` with a stubbed `hl` and return the window, workspace and layer rules it declares. Rules
/// that `path` pulls in from `skip` (the managed rules file) aren't counted.
pub fn evaluate_rules(path: &Path, skip: Option<&Path>) -> Result<ImportResult, String> {
    let lua = Lua::new();
    let skip = skip.and_then(|p| p.canonicalize().ok());
    let skipped = lua
        .create_function(move |_, file: String| {
            Ok(skip.is_some() && Path::new(&file).canonicalize().ok() == skip)
        })
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(10_000),
        move |_, _| {
            if start.elapsed() > TIMEOUT {
                Err(mlua::Error::runtime("timed out evaluating the file"))
            } else {
                Ok(VmState::Continue)
            }
        },
    )
    .map_err(|e| e.to_string())?;
    let dir = path
        .parent()
        .unwrap_or(Path::new("."))
        .to_string_lossy()
        .into_owned();
    let (specs, error): (mlua::Table, Option<String>) = lua
        .load(STUB)
        .set_name("hypr-rules import")
        .call((path.to_string_lossy().into_owned(), dir, skipped))
        .map_err(|e| e.to_string())?;
    let mut rules = Vec::new();
    for spec in specs.sequence_values::<mlua::Value>() {
        let Ok(entry) = spec else { continue };
        let Some(Value::Map(entry)) = convert(&entry, 0) else {
            continue;
        };
        let mut kind = RuleKind::Window;
        let mut spec = None;
        for (k, v) in entry {
            match (k.as_str(), v) {
                ("kind", Value::Str(s)) => {
                    kind = match s.as_str() {
                        "workspace" => RuleKind::Workspace,
                        "layer" => RuleKind::Layer,
                        _ => RuleKind::Window,
                    }
                }
                ("spec", Value::Map(m)) => spec = Some(m),
                _ => {}
            }
        }
        if let Some(m) = spec {
            rules.push(Rule::from_spec(kind, m));
        }
    }
    Ok(ImportResult { rules, error })
}

/// Lua value -> Value. Stub objects (tables with metatables), functions and deep nesting
/// have no place in a rule spec and are dropped.
fn convert(v: &mlua::Value, depth: usize) -> Option<Value> {
    match v {
        mlua::Value::Boolean(b) => Some(Value::Bool(*b)),
        mlua::Value::Integer(i) => Some(Value::Int(*i)),
        mlua::Value::Number(n) if n.is_finite() => Some(Value::Float(*n)),
        mlua::Value::String(s) => Some(Value::Str(s.to_string_lossy())),
        mlua::Value::Table(t) if depth < 16 && t.metatable().is_none() => {
            let n = t.raw_len();
            let pairs: Vec<(mlua::Value, mlua::Value)> = t.pairs().filter_map(Result::ok).collect();
            if n > 0 && pairs.len() == n {
                let items = (1..=n)
                    .filter_map(|i| t.raw_get::<mlua::Value>(i).ok())
                    .filter_map(|v| convert(&v, depth + 1));
                return Some(Value::List(items.collect()));
            }
            let mut map: Vec<(String, Value)> = pairs
                .iter()
                .filter_map(|(k, v)| {
                    let key = match k {
                        mlua::Value::String(s) => s.to_string_lossy(),
                        mlua::Value::Integer(i) => i.to_string(),
                        _ => return None,
                    };
                    Some((key, convert(v, depth + 1)?))
                })
                .collect();
            map.sort_by(|a, b| a.0.cmp(&b.0));
            Some(Value::Map(map))
        }
        _ => None,
    }
}

// ---- locating hl.*_rule(...) calls in source text ---------------------------------

#[derive(Debug, Clone, Copy)]
pub struct CallSpan {
    /// Offset of "hl.window_rule" (or the workspace/layer variant).
    pub start: usize,
    /// Offset just past the closing ")".
    pub end: usize,
}

/// If src[i] opens a Lua long bracket ([[ or [==[), return the offset after its close.
fn skip_long_bracket(src: &str, i: usize) -> Option<usize> {
    let b = src.as_bytes();
    let mut j = i + 1;
    while j < b.len() && b[j] == b'=' {
        j += 1;
    }
    if j >= b.len() || b[j] != b'[' {
        return None;
    }
    let close = format!("]{}]", "=".repeat(j - i - 1));
    Some(
        src[j + 1..]
            .find(&close)
            .map_or(src.len(), |p| j + 1 + p + close.len()),
    )
}

fn skip_string(src: &str, i: usize) -> usize {
    let b = src.as_bytes();
    let quote = b[i];
    let mut j = i + 1;
    while j < b.len() {
        if b[j] == b'\\' {
            j += 2;
            continue;
        }
        if b[j] == quote || b[j] == b'\n' {
            return j + 1;
        }
        j += 1;
    }
    b.len()
}

fn skip_line(src: &str, i: usize) -> usize {
    src.as_bytes()[i..]
        .iter()
        .position(|&c| c == b'\n')
        .map_or(src.len(), |p| i + p + 1)
}

/// Find every hl.window_rule( ... ), hl.workspace_rule( ... ) and hl.layer_rule( ... ) call
/// outside comments and strings, in source order.
// Scans bytes, never str slices: `i` steps through multi-byte characters (a BOM, say).
pub fn find_window_rule_calls(src: &str) -> Vec<CallSpan> {
    const NEEDLES: [&[u8]; 3] = [b"hl.window_rule", b"hl.workspace_rule", b"hl.layer_rule"];
    let b = src.as_bytes();
    let n = b.len();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < n {
        let c = b[i];
        if b[i..].starts_with(b"--") {
            if i + 2 < n
                && b[i + 2] == b'['
                && let Some(end) = skip_long_bracket(src, i + 2)
            {
                i = end;
                continue;
            }
            i = skip_line(src, i);
            continue;
        }
        if c == b'"' || c == b'\'' {
            i = skip_string(src, i);
            continue;
        }
        if c == b'['
            && let Some(end) = skip_long_bracket(src, i)
        {
            i = end;
            continue;
        }
        let boundary = i == 0
            || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] >= 0x80 || b"_.".contains(&b[i - 1]));
        let needle = NEEDLES.iter().find(|n| b[i..].starts_with(n));
        if boundary && let Some(needle) = needle {
            let mut j = i + needle.len();
            while j < n && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < n
                && b[j] == b'('
                && let Some(end) = match_paren(src, j)
            {
                spans.push(CallSpan { start: i, end });
                i = end;
                continue;
            }
        }
        i += 1;
    }
    spans
}

fn match_paren(src: &str, mut i: usize) -> Option<usize> {
    let b = src.as_bytes();
    let mut depth = 0;
    while i < b.len() {
        let c = b[i];
        if b[i..].starts_with(b"--") {
            i = skip_line(src, i);
            continue;
        }
        if c == b'"' || c == b'\'' {
            i = skip_string(src, i);
            continue;
        }
        if c == b'['
            && let Some(end) = skip_long_bracket(src, i)
        {
            i = end;
            continue;
        }
        if c == b'(' {
            depth += 1;
        } else if c == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

/// Whether every call has its lines to itself (bar indentation, a `;` and a trailing
/// comment), so commenting out those whole lines can't take other code with it.
pub fn calls_on_own_lines(src: &str, spans: &[CallSpan]) -> bool {
    spans.iter().all(|span| {
        let line_start = src[..span.start].rfind('\n').map_or(0, |p| p + 1);
        let line_end = src[span.end..]
            .find('\n')
            .map_or(src.len(), |p| span.end + p);
        let after = src[span.end..line_end].trim_start();
        let after = after.strip_prefix(';').unwrap_or(after).trim_start();
        src[line_start..span.start].trim().is_empty()
            && (after.is_empty() || (after.starts_with("--") && !after.starts_with("--[")))
    })
}

/// Comment out the given hl.*_rule calls (by source order) in `path`. Returns the backup path.
pub fn comment_out_calls(path: &Path, indices: &[usize]) -> io::Result<PathBuf> {
    let mut src = fs::read_to_string(path)?;
    let spans = find_window_rule_calls(&src);
    let backup = backup_file(path)?;
    let mut sorted: Vec<usize> = indices.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    for idx in sorted {
        let Some(span) = spans.get(idx) else { continue };
        let line_start = src[..span.start].rfind('\n').map_or(0, |p| p + 1);
        let line_end = src[span.end..]
            .find('\n')
            .map_or(src.len(), |p| span.end + p);
        let commented: Vec<String> = src[line_start..line_end]
            .split('\n')
            .map(|l| {
                if l.trim().is_empty() {
                    l.to_string()
                } else {
                    format!("-- {l}")
                }
            })
            .collect();
        let replacement = format!("-- [moved to hypr-rules]\n{}", commented.join("\n"));
        src.replace_range(line_start..line_end, &replacement);
    }
    atomic_write(path, &src)?;
    Ok(backup)
}

// ---- writing ---------------------------------------------------------------------------

const HEADER: &str = "-- Managed by hypr-rules — edit with the app.\n\
-- Hand edits are fine as long as this stays a list of hl.*_rule({...}) calls; `-- @group` lines are the app's groups.\n";

/// Write `rules` to `path` (backing up any previous version). Returns the backup path.
#[cfg(test)]
pub fn write_rules(path: &Path, rules: &[Rule]) -> io::Result<Option<PathBuf>> {
    write_rules_grouped(path, rules, &[])
}

/// Write `rules` to `path` (backing up any previous version), with `groups` written as comment markers.
pub fn write_rules_grouped(
    path: &Path,
    rules: &[Rule],
    groups: &[Group],
) -> io::Result<Option<PathBuf>> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let backup = if path.exists() {
        Some(backup_file(path)?)
    } else {
        None
    };
    atomic_write(path, &render_rules(rules, groups))?;
    Ok(backup)
}

const GROUP_MARK: &str = "-- @group";
const GROUP_OFF_MARK: &str = "-- @group-off";
const RULE_OFF_MARK: &str = "-- @rule-off";

/// The file text for `rules`. Group markers are comments, so Hyprland ignores them; a
/// disabled group writes `enabled = false` on its rules.
pub fn render_rules(rules: &[Rule], groups: &[Group]) -> String {
    let mut body: Vec<String> = Vec::with_capacity(rules.len());
    let mut prev = "";
    for rule in rules {
        let group_off =
            !rule.group.is_empty() && groups.iter().any(|g| g.name == rule.group && !g.enabled);
        let mut chunk = String::new();
        if rule.group != prev {
            let name = rule.group.replace(['\n', '\r'], " ");
            chunk.push_str(&match (rule.group.is_empty(), group_off) {
                (true, _) => format!("{GROUP_MARK}\n"),
                (false, true) => format!("{GROUP_OFF_MARK} {name}\n"),
                (false, false) => format!("{GROUP_MARK} {name}\n"),
            });
            prev = &rule.group;
        }
        if group_off {
            if !rule.enabled {
                chunk.push_str(RULE_OFF_MARK);
                chunk.push('\n');
            }
            let mut off = rule.clone();
            off.enabled = false;
            chunk.push_str(&lua_call(&off));
        } else {
            chunk.push_str(&lua_call(rule));
        }
        body.push(chunk);
    }
    format!("{HEADER}\n{}", body.join("\n"))
}

pub fn lua_call(rule: &Rule) -> String {
    format!(
        "hl.{}({})\n",
        rule.kind.lua_fn(),
        lua_value(&Value::Map(rule.to_spec()), 0)
    )
}

/// What [`read_rules`] found in a rules file.
pub struct RulesFile {
    pub rules: Vec<Rule>,
    pub groups: Vec<Group>,
    pub error: Option<String>,
}

/// Evaluate the rules file at `path` and recover its groups from the markers.
pub fn read_rules(path: &Path) -> Result<RulesFile, String> {
    let ImportResult { mut rules, error } = evaluate_rules(path, None)?;
    let src = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let groups = parse_groups(&src, &mut rules);
    Ok(RulesFile {
        rules,
        groups,
        error,
    })
}

/// Assign `rule.group` (and, in disabled groups, `enabled`) from the markers in `src`.
/// A hand-edited file whose call count differs from `rules` gets no groups.
pub fn parse_groups(src: &str, rules: &mut [Rule]) -> Vec<Group> {
    let spans = find_window_rule_calls(src);
    if spans.len() != rules.len() {
        return Vec::new();
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut current: Option<(String, bool)> = None;
    let mut cursor = 0;
    for (rule, span) in rules.iter_mut().zip(&spans) {
        let mut rule_off = false;
        for line in src[cursor..span.start].lines() {
            let line = line.trim();
            if line == RULE_OFF_MARK {
                rule_off = true;
            } else if line == GROUP_MARK {
                current = None;
            } else if let Some(name) = line.strip_prefix("-- @group-off ") {
                current = Some((name.trim().to_string(), false));
            } else if let Some(name) = line.strip_prefix("-- @group ") {
                current = Some((name.trim().to_string(), true));
            }
        }
        cursor = span.end;
        let Some((name, enabled)) = &current else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        rule.group = name.clone();
        if !enabled {
            rule.enabled = !rule_off;
        }
        if !groups.iter().any(|g| g.name == *name) {
            groups.push(Group {
                name: name.clone(),
                enabled: *enabled,
            });
        }
    }
    groups
}

/// One thing the file check found; `rule` is an index into the checked rules.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub rule: Option<usize>,
    pub message: String,
}

/// Render the rules, read the text back with the import stub and report whatever
/// Hyprland would reject, drop or read differently.
pub fn validate(rules: &[Rule], groups: &[Group]) -> Vec<Problem> {
    let mut problems = Vec::new();
    let whole = |message: String| Problem {
        rule: None,
        message,
    };
    let src = render_rules(rules, groups);
    // The counter keeps concurrent checks (tests, say) off each other's file
    static CHECKS: AtomicUsize = AtomicUsize::new(0);
    let n = CHECKS.fetch_add(1, Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!("hypr-rules-check-{}-{n}.lua", std::process::id()));
    let evaluated = fs::write(&tmp, &src)
        .map_err(|e| e.to_string())
        .and_then(|()| evaluate_rules(&tmp, None));
    let _ = fs::remove_file(&tmp);
    match evaluated {
        Err(e) => problems.push(whole(format!("The rules file doesn't evaluate: {e}"))),
        Ok(mut res) => {
            if let Some(e) = res.error {
                problems.push(whole(format!("The rules file fails when run: {e}")));
            }
            if res.rules.len() == rules.len() {
                parse_groups(&src, &mut res.rules);
                for (i, (rule, back)) in rules.iter().zip(&res.rules).enumerate() {
                    if canonical(rule) != canonical(back) {
                        problems.push(Problem {
                            rule: Some(i),
                            message: format!(
                                "Rule {} doesn't read back the same: “{}”",
                                i + 1,
                                rule.title()
                            ),
                        });
                    }
                }
            } else {
                problems.push(whole(format!(
                    "The file reads back {} rules instead of {}",
                    res.rules.len(),
                    rules.len()
                )));
            }
        }
    }
    for (i, rule) in rules.iter().enumerate() {
        for (key, _) in &rule.effects {
            if model::prop_info(rule.kind, key).is_none() {
                problems.push(Problem {
                    rule: Some(i),
                    message: format!("“{}”: unknown property `{key}`", rule.title()),
                });
            }
        }
        if !rule.has_condition() {
            problems.push(Problem {
                rule: Some(i),
                message: format!("“{}”: matches nothing", rule.title()),
            });
        }
    }
    problems
}

/// A rule's identity as the file stores it: key order and empty match rows don't count.
fn canonical(rule: &Rule) -> (RuleKind, &str, Vec<(String, Value)>) {
    fn sorted(value: Value) -> Value {
        match value {
            Value::Map(m) => Value::Map(sort_map(m)),
            other => other,
        }
    }
    fn sort_map(m: Vec<(String, Value)>) -> Vec<(String, Value)> {
        let mut m: Vec<_> = m.into_iter().map(|(k, v)| (k, sorted(v))).collect();
        m.sort_by(|a, b| a.0.cmp(&b.0));
        m
    }
    (rule.kind, &rule.group, sort_map(rule.to_spec()))
}

pub fn lua_value(value: &Value, indent: usize) -> String {
    match value {
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{f:.1}"),
        Value::Float(f) => f.to_string(),
        Value::Str(s) => lua_string(s),
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(|v| lua_value(v, indent + 1)).collect();
            format!("{{ {} }}", parts.join(", "))
        }
        Value::Map(m) if m.is_empty() => "{}".into(),
        Value::Map(m) => {
            let pad = "\t".repeat(indent);
            let inner = "\t".repeat(indent + 1);
            let lines: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("{inner}{} = {},", lua_key(k), lua_value(v, indent + 1)))
                .collect();
            format!("{{\n{}\n{pad}}}", lines.join("\n"))
        }
    }
}

const LUA_KEYWORDS: [&str; 22] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

fn lua_key(key: &str) -> String {
    let ident = key
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !LUA_KEYWORDS.contains(&key);
    if ident {
        key.to_string()
    } else {
        format!("[{}]", lua_string(key))
    }
}

pub fn lua_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            // A raw \r ends a Lua string just like \n; three digits so a following digit can't join in
            c if c.is_ascii_control() => out.push_str(&format!("\\{:03}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn backup_file(path: &Path) -> io::Result<PathBuf> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let backup = path.with_file_name(format!("{name}.{stamp}.bak"));
    fs::copy(path, &backup)?;
    prune_backups(path, KEEP_BACKUPS);
    Ok(backup)
}

const KEEP_BACKUPS: usize = 10;

/// Backups of `path` next to it, oldest first (the timestamp in the name sorts
/// chronologically).
fn backup_paths(path: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("{}.", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut backups: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".bak"))
        })
        .collect();
    backups.sort();
    backups
}

/// A saved earlier version of the rules file.
#[derive(Debug, Clone, PartialEq)]
pub struct Backup {
    pub path: PathBuf,
    /// "2026-11-24 13:05:09", from the file name.
    pub stamp: String,
}

/// Backups of `path`, newest first.
pub fn list_backups(path: &Path) -> Vec<Backup> {
    let mut backups: Vec<Backup> = backup_paths(path)
        .into_iter()
        .map(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            let stamp = name
                .as_deref()
                .and_then(|n| n.strip_suffix(".bak"))
                .and_then(|n| n.rsplit('.').next())
                .and_then(|t| chrono::NaiveDateTime::parse_from_str(t, "%Y%m%d-%H%M%S").ok())
                .map_or_else(
                    || name.clone().unwrap_or_default(),
                    |t| t.format("%Y-%m-%d %H:%M:%S").to_string(),
                );
            Backup { path: p, stamp }
        })
        .collect();
    backups.reverse();
    backups
}

/// Replace `path` with the text of `backup`, backing up the current file first.
pub fn restore_backup(path: &Path, backup: &Path) -> io::Result<Option<PathBuf>> {
    let text = fs::read_to_string(backup)?;
    let saved = if path.exists() {
        Some(backup_file(path)?)
    } else {
        None
    };
    atomic_write(path, &text)?;
    Ok(saved)
}

/// Deletes all but the newest `keep` backups of `path`.
fn prune_backups(path: &Path, keep: usize) {
    let backups = backup_paths(path);
    let excess = backups.len().saturating_sub(keep);
    for old in &backups[..excess] {
        let _ = fs::remove_file(old);
    }
}

fn atomic_write(path: &Path, text: &str) -> io::Result<()> {
    // Write through a symlink (dotfile managers) instead of replacing it with a plain file
    let resolved = fs::canonicalize(path).ok();
    let path = resolved.as_deref().unwrap_or(path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_calls_outside_comments() {
        let src = "-- hl.window_rule({})\nhl.window_rule({ match = { class = \"a)\" } })\n--[[ hl.window_rule({}) ]]\nhl.window_rule ({})";
        let spans = find_window_rule_calls(src);
        assert_eq!(spans.len(), 2);
        assert!(src[spans[0].start..spans[0].end].ends_with("} })"));
    }

    #[test]
    fn scans_files_with_non_ascii_code() {
        let src = "\u{feff}local é = 1\nhl.window_rule({ name = \"ü\" })";
        assert_eq!(find_window_rule_calls(src).len(), 1);
    }

    #[test]
    fn comments_out_only_calls_alone_on_their_lines() {
        let own = "x = 1\n  hl.window_rule({\n})  ; -- note\nhl.window_rule({})\n";
        assert!(calls_on_own_lines(own, &find_window_rule_calls(own)));
        for shared in [
            "hl.monitor({}) hl.window_rule({})",
            "hl.window_rule({}) hl.bind(\"x\")",
            "hl.window_rule({}) hl.window_rule({})",
        ] {
            assert!(
                !calls_on_own_lines(shared, &find_window_rule_calls(shared)),
                "{shared}"
            );
        }
    }

    #[test]
    fn strings_with_control_characters_stay_valid_lua() {
        let lua = Lua::new();
        let s = "a\rb\0c\u{1b}1\"\\\n";
        let back: String = lua
            .load(format!("return {}", lua_string(s)))
            .eval()
            .unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn import_skips_rules_from_the_managed_file() {
        let dir = std::env::temp_dir().join(format!("hypr-rules-skip-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let managed = dir.join("managed.lua");
        fs::write(&managed, "hl.window_rule({ name = \"managed\" })").unwrap();
        let config = dir.join("config.lua");
        fs::write(
            &config,
            format!(
                "hl.window_rule({{ name = \"own\" }})\nrequire(\"managed\")\ndofile({})",
                lua_string(&managed.to_string_lossy())
            ),
        )
        .unwrap();
        let names = |skip| {
            evaluate_rules(&config, skip)
                .unwrap()
                .rules
                .into_iter()
                .map(|r| r.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(Some(&managed)), ["own"]);
        assert_eq!(names(None), ["own", "managed", "managed"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn round_trips_through_lua() {
        let dir = std::env::temp_dir().join(format!("hypr-rules-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rules.lua");
        let src = "hl.monitor({}) hl.window_rule({ name = \"x\", match = { class = \"^(steam)$\" }, float = true, size = { \"10\", 20 }, scrolling_width = 0.5, opacity = 1.0 })";
        fs::write(&path, src).unwrap();
        let res = evaluate_rules(&path, None).unwrap();
        assert!(res.error.is_none());
        assert_eq!(res.rules.len(), 1);
        let out = lua_call(&res.rules[0]);
        assert!(out.contains("class = \"^(steam)$\""), "{out}");
        assert!(out.contains("size = { \"10\", 20 }"), "{out}");
        assert!(out.contains("opacity = 1.0"), "{out}");
        fs::write(&path, &out).unwrap();
        assert_eq!(evaluate_rules(&path, None).unwrap().rules, res.rules);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keeps_only_the_newest_backups() {
        let dir = std::env::temp_dir().join(format!("hypr-rules-prune-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hypr-rules.lua");
        fs::write(&path, "x").unwrap();
        for i in 0..13 {
            fs::write(
                dir.join(format!("hypr-rules.lua.20260101-0000{i:02}.bak")),
                "",
            )
            .unwrap();
        }
        fs::write(dir.join("other.lua.20260101-000000.bak"), "").unwrap();
        prune_backups(&path, 10);
        let left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            left.iter()
                .filter(|n| n.to_string_lossy().starts_with("hypr-rules.lua."))
                .count(),
            10
        );
        assert!(!dir.join("hypr-rules.lua.20260101-000000.bak").exists());
        assert!(dir.join("hypr-rules.lua.20260101-000012.bak").exists());
        assert!(dir.join("other.lua.20260101-000000.bak").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hypr-rules-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn window(name: &str, class: &str, group: &str) -> Rule {
        let mut m = crate::model::MatchField::new("class");
        m.text = class.into();
        Rule {
            name: name.into(),
            matches: vec![m],
            effects: vec![("float".into(), Value::Bool(true))],
            group: group.into(),
            ..Default::default()
        }
    }

    fn workspace(sel: &str) -> Rule {
        Rule {
            kind: RuleKind::Workspace,
            effects: vec![
                ("workspace".into(), Value::Str(sel.into())),
                ("gaps_out".into(), Value::Int(0)),
            ],
            ..Default::default()
        }
    }

    fn layer() -> Rule {
        let mut m = crate::model::MatchField::new("namespace");
        m.text = "waybar".into();
        Rule {
            kind: RuleKind::Layer,
            matches: vec![m],
            effects: vec![("blur".into(), Value::Bool(true))],
            ..Default::default()
        }
    }

    #[test]
    fn all_three_kinds_round_trip() {
        let dir = temp_dir("kinds");
        let path = dir.join("rules.lua");
        let rules = vec![window("a", "kitty", ""), workspace("w[tv1]"), layer()];
        write_rules(&path, &rules).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("hl.workspace_rule(") && text.contains("hl.layer_rule("));
        let back = read_rules(&path).unwrap();
        assert!(back.error.is_none());
        assert_eq!(back.rules.len(), 3);
        assert_eq!(
            back.rules.iter().map(|r| r.kind).collect::<Vec<_>>(),
            [RuleKind::Window, RuleKind::Workspace, RuleKind::Layer]
        );
        assert_eq!(back.rules[1].to_spec(), rules[1].to_spec());
        assert_eq!(back.rules[2].matches[0].key, "namespace");
        assert!(back.groups.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finds_workspace_and_layer_calls() {
        let src = "hl.workspace_rule({ workspace = \"1\" })\nhl.layer_rule({})\nxhl.layer_rule({})";
        assert_eq!(find_window_rule_calls(src).len(), 2);
    }

    #[test]
    fn groups_round_trip() {
        let dir = temp_dir("groups");
        let path = dir.join("rules.lua");
        let mut rules = vec![
            window("free1", "a", ""),
            window("g1", "b", "Games"),
            window("g2", "c", "Games"),
            window("free2", "d", ""),
            window("o1", "e", "Old"),
            window("o2", "f", "Old"),
            window("o3", "g", "Old"),
        ];
        rules[5].enabled = false;
        let groups = vec![
            Group {
                name: "Games".into(),
                enabled: true,
            },
            Group {
                name: "Old".into(),
                enabled: false,
            },
        ];
        write_rules_grouped(&path, &rules, &groups).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("-- @group Games\n"), "{text}");
        assert!(text.contains("-- @group-off Old\n"), "{text}");
        assert!(text.contains("-- @rule-off\n"), "{text}");
        let back = read_rules(&path).unwrap();
        assert_eq!(back.groups, groups);
        let key = |rs: &[Rule]| {
            rs.iter()
                .map(|r| (r.name.clone(), r.group.clone(), r.enabled))
                .collect::<Vec<_>>()
        };
        assert_eq!(key(&back.rules), key(&rules));
        // Markers are only comments: evaluating without them yields the same rule count
        assert!(validate(&rules, &groups).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn markers_are_ignored_when_the_call_count_differs() {
        let src = "-- @group X\nhl.window_rule({})\nhl.window_rule({})";
        let mut rules = vec![Rule::default()];
        assert!(parse_groups(src, &mut rules).is_empty());
        assert_eq!(rules[0].group, "");
    }

    #[test]
    fn validate_flags_problems() {
        let clean = vec![window("a", "kitty", ""), workspace("2"), layer()];
        assert!(validate(&clean, &[]).is_empty());
        let mut bad = clean.clone();
        bad[0].effects.push(("flaot".into(), Value::Bool(true)));
        bad[1].effects.retain(|(k, _)| k != "workspace");
        let problems = validate(&bad, &[]);
        assert!(
            problems
                .iter()
                .any(|p| p.rule == Some(0) && p.message.contains("unknown property `flaot`"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p.rule == Some(1) && p.message.contains("matches nothing"))
        );
        assert!(!problems.iter().any(|p| p.rule == Some(2)));
    }

    #[test]
    fn lists_and_restores_backups() {
        let dir = temp_dir("backups");
        let path = dir.join("hypr-rules.lua");
        fs::write(&path, "current").unwrap();
        for (stamp, text) in [("20260101-000001", "old"), ("20260301-130509", "newer")] {
            fs::write(dir.join(format!("hypr-rules.lua.{stamp}.bak")), text).unwrap();
        }
        fs::write(dir.join("other.lua.20260401-000000.bak"), "x").unwrap();
        let list = list_backups(&path);
        assert_eq!(
            list.iter().map(|b| b.stamp.as_str()).collect::<Vec<_>>(),
            ["2026-03-01 13:05:09", "2026-01-01 00:00:01"]
        );
        let saved = restore_backup(&path, &list[1].path).unwrap().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(fs::read_to_string(saved).unwrap(), "current");
        fs::remove_dir_all(&dir).unwrap();
    }
}
