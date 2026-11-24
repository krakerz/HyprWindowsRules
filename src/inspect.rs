//! Rule analysis against open windows: findings, effective result, apply expressions.

use serde_json::Value as Json;

use crate::luaio::{lua_string, lua_value};
use crate::model::{Rule, RuleKind, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum FindingKind {
    NoOpenMatch,
    CatchAll,
    Overridden { by: usize, key: String },
    Conflict { with: usize, key: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub rule: usize,
    pub kind: FindingKind,
}

fn applies(rule: &Rule, client: &Json) -> bool {
    rule.enabled && rule.matches_window(client)
}

/// A `.*` / `.+` pattern on its own matches every window.
fn catch_all(rule: &Rule) -> bool {
    let patterns: Vec<String> = rule
        .matches
        .iter()
        .filter(|m| m.is_pattern() && (!m.text.is_empty() || m.raw.is_some()))
        .map(|m| m.pattern())
        .collect();
    !patterns.is_empty() && patterns.iter().all(|p| p == ".*" || p == ".+")
}

fn is_true(rule: &Rule, key: &str) -> bool {
    rule.effect(key).is_some_and(Value::truthy)
}

/// Overlaps, overridden effects and rules that match nothing, for enabled window rules.
pub fn analyze(rules: &[Rule], clients: &[Json]) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    let mut push = |f: Finding| {
        if !out.contains(&f) {
            out.push(f);
        }
    };
    for (i, rule) in rules.iter().enumerate() {
        if !rule.enabled || rule.kind != RuleKind::Window || !rule.has_condition() {
            continue;
        }
        if catch_all(rule) {
            push(Finding {
                rule: i,
                kind: FindingKind::CatchAll,
            });
        }
        // With no windows to test against (Hyprland unreachable) there is nothing to say
        if !clients.is_empty() && !clients.iter().any(|c| rule.matches_window(c)) {
            push(Finding {
                rule: i,
                kind: FindingKind::NoOpenMatch,
            });
        }
    }
    for client in clients {
        let hits: Vec<usize> = (0..rules.len())
            .filter(|&i| applies(&rules[i], client))
            .collect();
        for (n, &a) in hits.iter().enumerate() {
            for &b in &hits[n + 1..] {
                for (key, va) in &rules[a].effects {
                    if let Some(vb) = rules[b].effect(key)
                        && va != vb
                    {
                        push(Finding {
                            rule: a,
                            kind: FindingKind::Overridden {
                                by: b,
                                key: key.clone(),
                            },
                        });
                    }
                }
                let clash = (is_true(&rules[a], "float") && is_true(&rules[b], "tile"))
                    || (is_true(&rules[a], "tile") && is_true(&rules[b], "float"));
                if clash {
                    for (x, y) in [(a, b), (b, a)] {
                        push(Finding {
                            rule: x,
                            kind: FindingKind::Conflict {
                                with: y,
                                key: "float".into(),
                            },
                        });
                    }
                }
            }
        }
    }
    out
}

/// What a window ends up with: for each effect the last matching rule wins.
/// Returns `(key, value, rule index)` in first-seen key order.
pub fn effective(rules: &[Rule], client: &Json) -> Vec<(String, Value, usize)> {
    let mut out: Vec<(String, Value, usize)> = Vec::new();
    for (i, rule) in rules.iter().enumerate().filter(|(_, r)| applies(r, client)) {
        for (key, value) in &rule.effects {
            match out.iter_mut().find(|(k, ..)| k == key) {
                Some(slot) => {
                    slot.1 = value.clone();
                    slot.2 = i;
                }
                None => out.push((key.clone(), value.clone(), i)),
            }
        }
    }
    out
}

/// The window argument of every dispatcher. The `window = "address:…"` key is not
/// confirmed against Hyprland 0.56 yet; keep it in this one place.
fn window_arg(client: &Json) -> String {
    let address = crate::hypr::address(client);
    format!("window = {}", lua_string(&format!("address:{address}")))
}

fn dsp(name: &str, fields: &[String], client: &Json) -> String {
    let mut all: Vec<String> = fields.to_vec();
    all.push(window_arg(client));
    format!("hl.dsp.window.{name}({{ {} }})", all.join(", "))
}

fn toggle(name: &str, on: bool, client: &Json) -> String {
    let action = if on { "enable" } else { "disable" };
    dsp(name, &[format!("action = \"{action}\"")], client)
}

/// "W H" given as a list or one string.
fn pair(v: &Value) -> Option<(String, String)> {
    match v {
        Value::List(items) if items.len() == 2 => Some((items[0].plain(), items[1].plain())),
        Value::Str(s) => {
            let mut it = s.split_whitespace();
            let pair = (it.next()?.to_string(), it.next()?.to_string());
            it.next().is_none().then_some(pair)
        }
        _ => None,
    }
}

/// "50%" of `total`, or a plain pixel count.
fn pixels(text: &str, total: i64) -> Option<i64> {
    let text = text.trim();
    match text.strip_suffix('%') {
        Some(p) => Some((total as f64 * p.trim().parse::<f64>().ok()? / 100.0).round() as i64),
        None => Some(text.parse::<f64>().ok()?.round() as i64),
    }
}

fn monitor_size(client: &Json, monitors: &[Json]) -> Option<(i64, i64)> {
    let id = client.get("monitor").and_then(Json::as_i64);
    let m = monitors
        .iter()
        .find(|m| m.get("id").and_then(Json::as_i64) == id)?;
    let n = |k: &str| m.get(k).and_then(Json::as_i64);
    Some((n("width")?, n("height")?))
}

/// `hyprctl dispatch` Lua expressions that give an open window the rule's effects
/// right away (rules otherwise only apply when a window opens).
pub fn apply_exprs(rule: &Rule, client: &Json, monitors: &[Json]) -> Vec<String> {
    if rule.kind != RuleKind::Window || !rule.enabled {
        return Vec::new();
    }
    let size = monitor_size(client, monitors);
    let mut out = Vec::new();
    for (key, value) in &rule.effects {
        match (key.as_str(), value) {
            ("float", Value::Bool(b)) => out.push(toggle("float", *b, client)),
            ("tile", Value::Bool(true)) => out.push(toggle("float", false, client)),
            ("pin", Value::Bool(b)) => out.push(toggle("pin", *b, client)),
            ("center", Value::Bool(true)) => out.push(dsp("center", &[], client)),
            ("size" | "move", v) => {
                let Some(((a, b), (w, h))) = pair(v).zip(size) else {
                    continue;
                };
                let (Some(x), Some(y)) = (pixels(&a, w), pixels(&b, h)) else {
                    continue;
                };
                let name = if key == "size" { "resize" } else { "move" };
                let fields = [
                    format!("x = {x}"),
                    format!("y = {y}"),
                    "exact = true".into(),
                ];
                out.push(dsp(name, &fields, client));
            }
            ("workspace", v) => {
                let text = v.plain();
                let ws = text.strip_suffix(" silent").unwrap_or(&text).trim();
                let fields = [
                    format!("workspace = {}", lua_string(ws)),
                    "follow = false".into(),
                ];
                out.push(dsp("move", &fields, client));
            }
            (key, v) => {
                let fields = [
                    format!("prop = {}", lua_string(key)),
                    format!("value = {}", lua_value(v, 0)),
                ];
                out.push(dsp("set_prop", &fields, client));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MatchField;
    use serde_json::json;

    fn rule(class: &str, effects: &[(&str, Value)]) -> Rule {
        let mut m = MatchField::new("class");
        m.text = class.into();
        if class == ".*" {
            m.raw = Some(".*".into());
        }
        Rule {
            matches: vec![m],
            effects: effects
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            ..Default::default()
        }
    }

    fn client(class: &str) -> Json {
        json!({"class": class, "title": "t", "address": "0xabc", "monitor": 0})
    }

    fn monitors() -> Vec<Json> {
        vec![json!({"id": 0, "width": 1920, "height": 1080})]
    }

    #[test]
    fn effective_takes_the_last_writer() {
        let rules = vec![
            rule(
                ".*",
                &[
                    ("float", Value::Bool(true)),
                    ("opacity", Value::Str("0.9".into())),
                ],
            ),
            rule("kitty", &[("opacity", Value::Str("0.5".into()))]),
            rule("other", &[("pin", Value::Bool(true))]),
        ];
        let got = effective(&rules, &client("kitty"));
        assert_eq!(
            got,
            [
                ("float".into(), Value::Bool(true), 0),
                ("opacity".into(), Value::Str("0.5".into()), 1)
            ]
        );
    }

    #[test]
    fn analyze_finds_catch_all_override_and_no_match() {
        let rules = vec![
            rule(".*", &[("opacity", Value::Str("0.9".into()))]),
            rule("kitty", &[("opacity", Value::Str("0.5".into()))]),
            rule("gone", &[("float", Value::Bool(true))]),
        ];
        let f = analyze(&rules, &[client("kitty")]);
        assert!(f.contains(&Finding {
            rule: 0,
            kind: FindingKind::CatchAll
        }));
        assert!(f.contains(&Finding {
            rule: 0,
            kind: FindingKind::Overridden {
                by: 1,
                key: "opacity".into()
            }
        }));
        assert!(f.contains(&Finding {
            rule: 2,
            kind: FindingKind::NoOpenMatch
        }));
        assert!(!f.iter().any(|x| x.rule == 1));
        assert!(
            analyze(&rules, &[])
                .iter()
                .all(|x| x.kind == FindingKind::CatchAll)
        );
    }

    #[test]
    fn analyze_flags_float_tile_conflicts_once() {
        let rules = vec![
            rule("kitty", &[("float", Value::Bool(true))]),
            rule("kitty", &[("tile", Value::Bool(true))]),
        ];
        let f = analyze(&rules, &[client("kitty"), client("kitty")]);
        let conflicts = f
            .iter()
            .filter(|x| matches!(x.kind, FindingKind::Conflict { .. }))
            .count();
        assert_eq!(conflicts, 2);
    }

    #[test]
    fn apply_exprs_builds_dispatchers() {
        let r = rule(
            "kitty",
            &[
                ("float", Value::Bool(true)),
                (
                    "size",
                    Value::List(vec![Value::Str("50%".into()), Value::Int(300)]),
                ),
                ("move", Value::Str("10% 20".into())),
                ("workspace", Value::Str("2 silent".into())),
                ("opacity", Value::Str("0.8".into())),
            ],
        );
        let w = "window = \"address:0xabc\"";
        assert_eq!(
            apply_exprs(&r, &client("kitty"), &monitors()),
            [
                format!("hl.dsp.window.float({{ action = \"enable\", {w} }})"),
                format!("hl.dsp.window.resize({{ x = 960, y = 300, exact = true, {w} }})"),
                format!("hl.dsp.window.move({{ x = 192, y = 20, exact = true, {w} }})"),
                format!("hl.dsp.window.move({{ workspace = \"2\", follow = false, {w} }})"),
                format!("hl.dsp.window.set_prop({{ prop = \"opacity\", value = \"0.8\", {w} }})"),
            ]
        );
    }

    #[test]
    fn apply_exprs_skips_what_cannot_apply() {
        let mut r = rule(
            "kitty",
            &[
                ("size", Value::Str("50% 50%".into())),
                ("center", Value::Bool(true)),
            ],
        );
        // No monitor info: percentages can't be resolved, center still works
        assert_eq!(apply_exprs(&r, &client("kitty"), &[]).len(), 1);
        let c = apply_exprs(&r, &client("kitty"), &monitors());
        assert_eq!(c.len(), 2);
        assert!(c[1].starts_with("hl.dsp.window.center("));
        assert!(c[0].starts_with("hl.dsp.window.resize("));
        r.enabled = false;
        assert!(apply_exprs(&r, &client("kitty"), &monitors()).is_empty());
    }
}
