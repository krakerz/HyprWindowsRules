//! App settings and the hook line that makes Hyprland load the managed rules file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::luaio::lua_string;

pub const HOOK_COMMENT: &str = "-- Window rules managed by the hypr-rules app";
const HOOK_TAG: &str = "-- hypr-rules";

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| home().join(".config"))
        .join("hypr-rules")
}

/// Named after the app so `require("hypr-rules")` can't pick up some other rules.lua on the path.
pub fn default_rules() -> PathBuf {
    config_dir().join("hypr-rules.lua")
}

fn old_default_rules() -> PathBuf {
    config_dir().join("rules.lua")
}

/// Hyprland's main config: always loaded, and watched for changes.
pub fn default_hook() -> PathBuf {
    home().join(".config/hypr/hyprland.lua")
}

/// Earlier versions hooked into Caelestia's user file; installing the hook cleans these up.
fn legacy_hooks() -> Vec<PathBuf> {
    vec![home().join(".config/caelestia/hypr-user.lua")]
}

/// Files whose existing window rules are offered for import on first run.
pub fn import_candidates(hook: &Path) -> Vec<PathBuf> {
    let mut v = vec![hook.to_path_buf()];
    v.extend(legacy_hooks());
    v
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePref {
    pub const ALL: [ThemePref; 3] = [ThemePref::System, ThemePref::Light, ThemePref::Dark];
}

impl std::fmt::Display for ThemePref {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ThemePref::System => "System theme",
            ThemePref::Light => "Light",
            ThemePref::Dark => "Dark",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub rules_path: String,
    pub hook_file: String,
    pub reload_on_save: bool,
    pub theme: ThemePref,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            rules_path: default_rules().to_string_lossy().into_owned(),
            hook_file: default_hook().to_string_lossy().into_owned(),
            reload_on_save: true,
            theme: ThemePref::System,
        }
    }
}

impl Settings {
    fn file() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        let mut s: Settings = fs::read_to_string(Self::file())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if s.rules_path.trim().is_empty() {
            s.rules_path = Settings::default().rules_path;
        }
        if s.hook_file.trim().is_empty() {
            s.hook_file = Settings::default().hook_file;
        }
        s.migrate_rules_file();
        s
    }

    /// 0.2.0 renamed the managed file; move it and repoint an installed hook at the new name.
    fn migrate_rules_file(&mut self) {
        let (old, new) = (old_default_rules(), default_rules());
        if self.rules() != old && self.rules() != new {
            return;
        }
        self.rules_path = new.to_string_lossy().into_owned();
        if !old.exists() || new.exists() || fs::rename(&old, &new).is_err() {
            return;
        }
        let hook = self.hook();
        if fs::read_to_string(&hook)
            .is_ok_and(|t| t.lines().any(|l| l.trim_end().ends_with(HOOK_TAG)))
        {
            let _ = install_hook(&hook, &new);
        }
    }

    pub fn save(&self) -> io::Result<()> {
        fs::create_dir_all(config_dir())?;
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(Self::file(), text + "\n")
    }

    pub fn rules(&self) -> PathBuf {
        expand(&self.rules_path)
    }

    pub fn hook(&self) -> PathBuf {
        expand(&self.hook_file)
    }
}

pub fn expand(p: &str) -> PathBuf {
    let p = p.trim();
    match p.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if p == "~" => home(),
        None => PathBuf::from(p),
    }
}

/// Lua expression for `path`. `home_var` is the Lua expression for $HOME; paths under it are
/// written relative to it so configs stay portable.
fn lua_path(path: &Path, home_var: &str) -> String {
    match path.strip_prefix(home()) {
        Ok(rel) => format!(
            "{home_var} .. {}",
            lua_string(&format!("/{}", rel.to_string_lossy()))
        ),
        Err(_) => lua_string(&path.to_string_lossy()),
    }
}

/// Caelestia's hyprland.lua defines `home` and `maybe_create` at the top; a hook in that file
/// can use them like Caelestia's own `require("hypr-user")` does.
fn is_caelestia(text: &str) -> bool {
    text.lines().any(|l| l.starts_with("local home"))
        && text
            .lines()
            .any(|l| l.starts_with("local function maybe_create("))
}

/// The rules file is loaded with require() rather than dofile(): Hyprland watches files it
/// loads through require and reloads by itself when they change; dofile'd files aren't
/// watched, so edits only took effect after a reload that wasn't always triggered.
fn hook_lines_for(hook_text: &str, rules: &Path) -> Vec<String> {
    let dir = rules.parent().unwrap_or(Path::new("/"));
    let module = lua_string(
        &rules
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    if is_caelestia(hook_text) {
        // Appended to the path like Caelestia's own folder
        return vec![
            format!(
                "package.path = package.path .. \";\" .. {} {HOOK_TAG}",
                lua_path(&dir.join("?.lua"), "home")
            ),
            format!("maybe_create({}) {HOOK_TAG}", lua_path(rules, "home")),
            format!("require({module}) {HOOK_TAG}"),
        ];
    }
    // Self-contained for other configs: skips a missing file and restores package.path
    vec![format!(
        "do local d, m = {}, {module}; local f = io.open(d .. \"/\" .. m .. \".lua\") \
         if f then f:close(); local old = package.path; package.path = d .. \"/?.lua;\" .. old; \
         require(m); package.path = old end end {HOOK_TAG}",
        lua_path(dir, "os.getenv(\"HOME\")"),
    )]
}

/// The hook as it would be written into `hook` (its style depends on that file).
pub fn hook_text(hook: &Path, rules: &Path) -> String {
    hook_lines_for(&fs::read_to_string(hook).unwrap_or_default(), rules).join("\n")
}

pub fn hook_installed(hook: &Path, rules: &Path) -> bool {
    fs::read_to_string(hook).is_ok_and(|t| t.contains(&hook_lines_for(&t, rules).join("\n")))
}

fn strip_hook_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|l| !l.trim_end().ends_with(HOOK_TAG) && l.trim() != HOOK_COMMENT)
        .collect()
}

/// Append the hook lines to `hook`, replacing any earlier hypr-rules hook there or in the
/// files older versions used.
pub fn install_hook(hook: &Path, rules: &Path) -> io::Result<()> {
    for legacy in legacy_hooks() {
        if legacy == hook {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&legacy) {
            let kept = strip_hook_lines(&text);
            if kept.len() != text.lines().count() {
                let mut kept: Vec<&str> = kept;
                while kept.last().is_some_and(|l| l.trim().is_empty()) {
                    kept.pop();
                }
                fs::write(&legacy, kept.join("\n") + "\n")?;
            }
        }
    }
    let text = fs::read_to_string(hook).unwrap_or_default();
    let mut lines: Vec<String> = strip_hook_lines(&text)
        .into_iter()
        .map(String::from)
        .collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let hook_lines = hook_lines_for(&text, rules);
    lines.extend(["".into(), HOOK_COMMENT.into()]);
    lines.extend(hook_lines);
    lines.push("".into());
    if let Some(dir) = hook.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(hook, lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caelestia_style_hook() {
        let rules = home().join(".config/hypr-rules/hypr-rules.lua");
        let caelestia =
            "local home   = os.getenv(\"HOME\")\nlocal function maybe_create(file, content)\nend\n";
        assert_eq!(
            hook_lines_for(caelestia, &rules),
            [
                "package.path = package.path .. \";\" .. home .. \"/.config/hypr-rules/?.lua\" -- hypr-rules",
                "maybe_create(home .. \"/.config/hypr-rules/hypr-rules.lua\") -- hypr-rules",
                "require(\"hypr-rules\") -- hypr-rules",
            ]
        );
        assert_eq!(hook_lines_for("", &rules).len(), 1);
    }
}
