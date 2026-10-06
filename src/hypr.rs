//! Thin wrappers around hyprctl.

use std::process::Command;

use serde_json::Value as Json;

fn hyprctl(args: &[&str]) -> Result<String, String> {
    let out = Command::new("hyprctl")
        .args(args)
        .output()
        .map_err(|e| format!("hyprctl failed: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let msg = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        return Err(if msg.is_empty() {
            "hyprctl failed".into()
        } else {
            msg.into()
        });
    }
    Ok(stdout)
}

fn json(args: &[&str]) -> Result<Vec<Json>, String> {
    serde_json::from_str(&hyprctl(args)?).map_err(|e| format!("unexpected hyprctl output: {e}"))
}

/// Mapped windows, from `hyprctl clients -j`.
pub fn clients() -> Result<Vec<Json>, String> {
    Ok(json(&["clients", "-j"])?
        .into_iter()
        .filter(|c| c.get("mapped").and_then(Json::as_bool) != Some(false))
        .collect())
}

pub fn monitors() -> Result<Vec<Json>, String> {
    json(&["monitors", "-j"])
}

/// Names of the workspaces that currently exist, special ones included.
pub fn workspaces() -> Result<Vec<String>, String> {
    Ok(json(&["workspaces", "-j"])?
        .iter()
        .map(|w| str_field(w, "name").to_string())
        .filter(|n| !n.is_empty())
        .collect())
}

/// Whether Hyprland reloads by itself when a watched config file changes.
pub fn autoreload_enabled() -> bool {
    hyprctl(&["getoption", "misc.disable_autoreload"])
        .map(|out| {
            !out.lines()
                .any(|l| l.trim() == "bool: true" || l.trim() == "int: 1")
        })
        .unwrap_or(false)
}

pub fn reload() -> Result<(), String> {
    hyprctl(&["reload"]).map(|_| ())
}

pub fn config_errors() -> Result<Vec<String>, String> {
    Ok(hyprctl(&["configerrors"])?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

pub fn str_field<'a>(c: &'a Json, key: &str) -> &'a str {
    c.get(key).and_then(Json::as_str).unwrap_or("")
}

pub fn workspace_name(c: &Json) -> &str {
    c.get("workspace")
        .and_then(|w| w.get("name"))
        .and_then(Json::as_str)
        .unwrap_or("")
}

pub fn floating(c: &Json) -> bool {
    c.get("floating").and_then(Json::as_bool).unwrap_or(false)
}

fn pair(c: &Json, key: &str) -> (i64, i64) {
    let v = c.get(key).and_then(Json::as_array);
    let at = |i: usize| v.and_then(|a| a.get(i)).and_then(Json::as_i64).unwrap_or(0);
    (at(0), at(1))
}

pub fn size(c: &Json) -> (i64, i64) {
    pair(c, "size")
}

/// Window position relative to its monitor's top-left corner.
pub fn relative_position(c: &Json, monitors: &[Json]) -> (i64, i64) {
    let (x, y) = pair(c, "at");
    let id = c.get("monitor").and_then(Json::as_i64);
    match monitors
        .iter()
        .find(|m| m.get("id").and_then(Json::as_i64) == id)
    {
        Some(m) => (
            x - m.get("x").and_then(Json::as_i64).unwrap_or(0),
            y - m.get("y").and_then(Json::as_i64).unwrap_or(0),
        ),
        None => (x, y),
    }
}
