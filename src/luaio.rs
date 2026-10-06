//! Read window rules out of Hyprland Lua config files, and write them back.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mlua::{HookTriggers, Lua, VmState};

use crate::model::{Rule, Value};

// Runs the target file with a fake `hl` that records hl.window_rule() calls and turns every
// other hl.* access into a harmless no-op, so arbitrary Hyprland configs evaluate cleanly.
const STUB: &str = r#"
local target, dir = ...
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
hl.window_rule = function(spec)
  rules[#rules + 1] = spec
  return dummy()
end

-- Config files must not be able to run things (or exit this process) while being imported
os.execute = function() return nil end
os.exit = function() error("os.exit called") end
os.remove = function() return nil end
os.rename = function() return nil end
io.popen = nil
package.path = dir .. "/?.lua;" .. package.path
local real_require = require
require = function(name)
  local ok, mod = pcall(real_require, name)
  if ok then return mod end
  return dummy()
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

/// Evaluate `path` with a stubbed `hl` and return the window rules it declares.
pub fn evaluate_rules(path: &Path) -> Result<ImportResult, String> {
    let lua = Lua::new();
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
        .call((path.to_string_lossy().into_owned(), dir))
        .map_err(|e| e.to_string())?;
    let mut rules = Vec::new();
    for spec in specs.sequence_values::<mlua::Value>() {
        if let Ok(spec) = spec
            && let Some(Value::Map(m)) = convert(&spec, 0)
        {
            rules.push(Rule::from_spec(m));
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

// ---- locating hl.window_rule(...) calls in source text ---------------------------------

#[derive(Debug, Clone, Copy)]
pub struct CallSpan {
    /// Offset of "hl.window_rule".
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
    src[i..].find('\n').map_or(src.len(), |p| i + p + 1)
}

/// Find every hl.window_rule( ... ) call outside comments and strings, in source order.
pub fn find_window_rule_calls(src: &str) -> Vec<CallSpan> {
    const NEEDLE: &str = "hl.window_rule";
    let b = src.as_bytes();
    let n = b.len();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < n {
        let c = b[i];
        if src[i..].starts_with("--") {
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
        if boundary && src[i..].starts_with(NEEDLE) {
            let mut j = i + NEEDLE.len();
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
        if src[i..].starts_with("--") {
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

/// Comment out the given hl.window_rule calls (by source order) in `path`. Returns the backup path.
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
-- Hand edits are fine as long as this stays a list of hl.window_rule({...}) calls.\n";

/// Write `rules` to `path` (backing up any previous version). Returns the backup path.
pub fn write_rules(path: &Path, rules: &[Rule]) -> io::Result<Option<PathBuf>> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let backup = if path.exists() {
        Some(backup_file(path)?)
    } else {
        None
    };
    let body: Vec<String> = rules.iter().map(lua_call).collect();
    atomic_write(path, &format!("{HEADER}\n{}", body.join("\n")))?;
    Ok(backup)
}

pub fn lua_call(rule: &Rule) -> String {
    format!(
        "hl.window_rule({})\n",
        lua_value(&Value::Map(rule.to_spec()), 0)
    )
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
    let out = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t");
    format!("\"{out}\"")
}

pub fn backup_file(path: &Path) -> io::Result<PathBuf> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let backup = path.with_file_name(format!("{name}.{stamp}.bak"));
    fs::copy(path, &backup)?;
    Ok(backup)
}

fn atomic_write(path: &Path, text: &str) -> io::Result<()> {
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
    fn round_trips_through_lua() {
        let dir = std::env::temp_dir().join(format!("hypr-rules-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rules.lua");
        let src = "hl.monitor({}) hl.window_rule({ name = \"x\", match = { class = \"^(steam)$\" }, float = true, size = { \"10\", 20 }, scrolling_width = 0.5, opacity = 1.0 })";
        fs::write(&path, src).unwrap();
        let res = evaluate_rules(&path).unwrap();
        assert!(res.error.is_none());
        assert_eq!(res.rules.len(), 1);
        let out = lua_call(&res.rules[0]);
        assert!(out.contains("class = \"^(steam)$\""), "{out}");
        assert!(out.contains("size = { \"10\", 20 }"), "{out}");
        assert!(out.contains("opacity = 1.0"), "{out}");
        fs::write(&path, &out).unwrap();
        assert_eq!(evaluate_rules(&path).unwrap().rules, res.rules);
        fs::remove_dir_all(&dir).unwrap();
    }
}
