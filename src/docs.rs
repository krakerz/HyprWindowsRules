//! Property catalog: the built-in tables merged with the Hyprland wiki's rule pages, cached
//! per installed Hyprland version so the wiki is only fetched again after an upgrade.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;
use std::{fs, io};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::model::{self, Kind, RuleKind};
use crate::update;

/// Raw markdown of the wiki's rule pages. The wiki has no tags or version branches, so
/// `main` (which documents the latest Hyprland) is the only source.
const WIKI_BASE: &str =
    "https://raw.githubusercontent.com/hyprwm/hyprland-wiki/main/content/configuring/core/rules";

/// A parsed table with fewer rows than this is taken for a changed page layout: the fetch
/// is rejected (window) or nothing is marked removed (the others).
const fn min_rows(kind: RuleKind) -> usize {
    match kind {
        RuleKind::Window => 10,
        RuleKind::Workspace => 8,
        RuleKind::Layer => 5,
    }
}

/// Keys the wiki's table doesn't list but the editor needs (the workspace selector).
fn exempt(kind: RuleKind, key: &str) -> bool {
    kind == RuleKind::Workspace && key == "workspace"
}

/// One property row of a wiki table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WikiProp {
    pub key: String,
    pub kind: Kind,
    pub help: String,
    pub choices: Vec<String>,
    pub multi: bool,
}

/// What one successful fetch leaves on disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocsCache {
    pub version: String,
    pub fetched_at: String,
    pub source: String,
    pub window: Vec<WikiProp>,
    pub workspace: Vec<WikiProp>,
    pub layer: Vec<WikiProp>,
}

impl DocsCache {
    fn props(&self, kind: RuleKind) -> &[WikiProp] {
        match kind {
            RuleKind::Window => &self.window,
            RuleKind::Workspace => &self.workspace,
            RuleKind::Layer => &self.layer,
        }
    }
}

// ---- parsing ---------------------------------------------------------------------------

static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\([^)]*\)").expect("link pattern is valid"));
static NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9_]+$").expect("name pattern is valid"));
static WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z]+$").expect("word pattern is valid"));

/// Prefixes after which the description lists the allowed values.
const VALUE_MARKERS: [&str; 3] = ["modes:", "space-separated:", "sets content type:"];

/// `line` without `<!-- … -->` comments (which may span lines; `open` carries the state).
fn strip_comments(line: &str, open: &mut bool) -> String {
    let mut out = String::new();
    let mut rest = line;
    loop {
        if *open {
            match rest.find("-->") {
                Some(end) => {
                    rest = &rest[end + 3..];
                    *open = false;
                }
                None => return out,
            }
        } else {
            match rest.find("<!--") {
                Some(start) => {
                    out.push_str(&rest[..start]);
                    rest = &rest[start + 4..];
                    *open = true;
                }
                None => {
                    out.push_str(rest);
                    return out;
                }
            }
        }
    }
}

fn split_row(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|');
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner
        .replace("\\|", "\u{0}")
        .split('|')
        .map(|c| c.replace('\u{0}', "|").trim().to_string())
        .collect()
}

fn is_separator(cells: &[String]) -> bool {
    cells
        .iter()
        .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
}

fn kind_of(type_cell: &str) -> Kind {
    match type_cell.trim().to_ascii_lowercase().as_str() {
        "bool" => Kind::Bool,
        "int" => Kind::Int,
        "float" => Kind::Float,
        _ => Kind::Str,
    }
}

/// Markdown description → plain one-line help.
fn clean_help(desc: &str) -> String {
    let plain = LINK.replace_all(desc, "$1").replace(['`', '*'], "");
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The allowed values named in a description: backticked tokens after "Modes:",
/// "Space-separated:" or "Sets content type:", else (tonemap style) three or more
/// unquoted lower-case words in backticks of a string property.
fn extract_choices(desc: &str, kind: Kind) -> Vec<String> {
    let lower = desc.to_ascii_lowercase();
    let marker = VALUE_MARKERS
        .iter()
        .filter_map(|m| lower.find(m).map(|at| (at, m.len())))
        .min_by_key(|(at, _)| *at);
    let mut out: Vec<String> = Vec::new();
    let mut push = |token: &str| {
        let token = token.trim_matches('"').trim();
        if !token.is_empty() && !out.iter().any(|t| t == token) {
            out.push(token.to_string());
        }
    };
    if let Some((at, len)) = marker {
        let mut rest = &desc[at + len..];
        loop {
            rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
            for word in ["or ", "and "] {
                rest = rest.strip_prefix(word).unwrap_or(rest);
            }
            let Some(inner) = rest.strip_prefix('`') else {
                break;
            };
            let Some(end) = inner.find('`') else { break };
            push(&inner[..end]);
            rest = &inner[end + 1..];
        }
    } else if kind == Kind::Str {
        let tokens: Vec<&str> = desc.split('`').skip(1).step_by(2).collect();
        if tokens.len() >= 3 && tokens.iter().all(|t| WORD.is_match(t)) {
            tokens.into_iter().for_each(&mut push);
        }
    }
    out
}

/// The effect (or workspace rule) tables of one wiki page: header `| Effect |` or
/// `| Rule |`, name in the first column, type in the third. Match-prop tables are skipped.
pub fn parse_effects(md: &str) -> Vec<WikiProp> {
    #[derive(PartialEq)]
    enum Table {
        None,
        Header,
        Body,
    }
    let mut out: Vec<WikiProp> = Vec::new();
    let mut table = Table::None;
    let mut in_comment = false;
    for raw in md.lines() {
        let line = strip_comments(raw, &mut in_comment);
        let line = line.trim();
        if line.is_empty() && !raw.trim().is_empty() {
            continue; // comment-only line, possibly inside a table
        }
        if !line.starts_with('|') {
            table = Table::None;
            continue;
        }
        let cells = split_row(line);
        match table {
            Table::None => {
                let first = cells[0].to_ascii_lowercase();
                if first == "effect" || first == "rule" {
                    table = Table::Header;
                }
            }
            Table::Header => {
                table = if is_separator(&cells) {
                    Table::Body
                } else {
                    Table::None
                };
            }
            Table::Body => {
                let key = cells[0].replace('`', "");
                if cells.len() < 3 || !NAME.is_match(&key) || out.iter().any(|p| p.key == key) {
                    continue;
                }
                let kind = kind_of(&cells[2]);
                out.push(WikiProp {
                    help: clean_help(&cells[1]),
                    choices: extract_choices(&cells[1], kind),
                    multi: cells[1].to_ascii_lowercase().contains("space-separated"),
                    key,
                    kind,
                });
            }
        }
    }
    out
}

// ---- catalog ---------------------------------------------------------------------------

/// A property the editor can offer, from the built-in table and/or the wiki.
#[derive(Debug, Clone, PartialEq)]
pub struct PropEntry {
    pub key: String,
    pub kind: Kind,
    pub help: String,
    pub choices: Vec<String>,
    pub multi: bool,
    /// Built-in, but the wiki for the installed version no longer lists it.
    pub removed: bool,
}

impl PropEntry {
    fn from_builtin(p: &model::PropInfo) -> Self {
        Self {
            key: p.key.into(),
            kind: p.kind,
            help: p.help.into(),
            choices: p.choices.iter().map(|c| c.to_string()).collect(),
            multi: p.multi,
            removed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct DocsMeta {
    version: String,
    fetched_at: String,
}

/// All known properties per rule kind, as the editor and the file check see them.
#[derive(Debug, Clone, PartialEq)]
pub struct PropCatalog {
    window: Vec<PropEntry>,
    workspace: Vec<PropEntry>,
    layer: Vec<PropEntry>,
    docs: Option<DocsMeta>,
}

impl PropCatalog {
    /// The offline baseline: just the static tables.
    pub fn builtin() -> Self {
        Self::merged(None)
    }

    /// Built-in tables overlaid with the wiki: a wiki row replaces help, and choices
    /// (with `multi`) when it names any, of a known key and adds unknown keys. Built-in
    /// keys a (large enough) wiki table no longer lists are marked `removed`.
    pub fn merged(cache: Option<&DocsCache>) -> Self {
        let build = |kind: RuleKind| {
            let mut entries: Vec<PropEntry> = model::props_for(kind)
                .iter()
                .map(PropEntry::from_builtin)
                .collect();
            if let Some(cache) = cache {
                merge_kind(&mut entries, cache.props(kind), kind);
            }
            entries
        };
        Self {
            window: build(RuleKind::Window),
            workspace: build(RuleKind::Workspace),
            layer: build(RuleKind::Layer),
            docs: cache.map(|c| DocsMeta {
                version: c.version.clone(),
                fetched_at: c.fetched_at.clone(),
            }),
        }
    }

    pub fn props(&self, kind: RuleKind) -> &[PropEntry] {
        match kind {
            RuleKind::Window => &self.window,
            RuleKind::Workspace => &self.workspace,
            RuleKind::Layer => &self.layer,
        }
    }

    pub fn info(&self, kind: RuleKind, key: &str) -> Option<&PropEntry> {
        self.props(kind).iter().find(|p| p.key == key)
    }

    /// Soft warning for a property the installed Hyprland's docs don't list.
    pub fn removed_note(&self) -> String {
        let version = self
            .docs
            .as_ref()
            .map_or("installed", |d| d.version.as_str());
        format!("Not in the Hyprland {version} docs — may be ignored")
    }

    /// Settings dialog line.
    pub fn status(&self) -> String {
        match &self.docs {
            Some(d) => format!("Hyprland {} · fetched {}", d.version, d.fetched_at),
            None => "built-in list".into(),
        }
    }
}

fn merge_kind(entries: &mut Vec<PropEntry>, wiki: &[WikiProp], kind: RuleKind) {
    for w in wiki {
        if let Some(e) = entries.iter_mut().find(|e| e.key == w.key) {
            if !w.help.is_empty() {
                e.help.clone_from(&w.help);
            }
            if !w.choices.is_empty() {
                e.choices.clone_from(&w.choices);
                e.multi = w.multi;
            }
            e.removed = false;
        } else {
            entries.push(PropEntry {
                key: w.key.clone(),
                kind: w.kind,
                help: w.help.clone(),
                choices: w.choices.clone(),
                multi: w.multi,
                removed: false,
            });
        }
    }
    if wiki.len() >= min_rows(kind) {
        for e in entries.iter_mut() {
            e.removed = !exempt(kind, &e.key) && !wiki.iter().any(|w| w.key == e.key);
        }
    }
}

// ---- cache on disk ---------------------------------------------------------------------

/// `~/.cache/hypr-rules`.
pub fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("hypr-rules"))
}

fn cache_file(dir: &Path, version: &str) -> PathBuf {
    let safe: String = version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("props-{safe}.json"))
}

/// The cached docs for exactly this Hyprland version, if present and readable.
pub fn load_from(dir: &Path, version: &str) -> Option<DocsCache> {
    let text = fs::read_to_string(cache_file(dir, version)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save_to(dir: &Path, cache: &DocsCache) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(cache).map_err(io::Error::other)?;
    fs::write(cache_file(dir, &cache.version), text)
}

pub fn load(version: &str) -> Option<DocsCache> {
    load_from(&cache_dir()?, version)
}

pub fn save(cache: &DocsCache) -> io::Result<()> {
    let dir = cache_dir().ok_or_else(|| io::Error::other("no cache directory"))?;
    save_to(&dir, cache)
}

// ---- fetching --------------------------------------------------------------------------

async fn get(client: &reqwest::Client, page: &str) -> Result<String, String> {
    let url = format!("{WIKI_BASE}/{page}-rules.md");
    client
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("couldn't fetch {page}-rules.md: {e}"))?
        .text()
        .await
        .map_err(|e| format!("couldn't read {page}-rules.md: {e}"))
}

/// Download and parse the three rule pages for `version`.
pub async fn fetch(version: String) -> Result<DocsCache, String> {
    let client = reqwest::Client::builder()
        .user_agent(format!("hypr-rules/{}", update::current_version()))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let window = parse_effects(&get(&client, "window").await?);
    let workspace = parse_effects(&get(&client, "workspace").await?);
    let layer = parse_effects(&get(&client, "layer").await?);
    if window.len() < min_rows(RuleKind::Window) {
        return Err("the wiki's window-rules page has a layout this version can't read".into());
    }
    Ok(DocsCache {
        version,
        fetched_at: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        source: WIKI_BASE.into(),
        window,
        workspace,
        layer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: &str = include_str!("../tests/fixtures/window-rules.md");
    const LAYER: &str = include_str!("../tests/fixtures/layer-rules.md");
    const WORKSPACE: &str = include_str!("../tests/fixtures/workspace-rules.md");

    fn find<'a>(props: &'a [WikiProp], key: &str) -> &'a WikiProp {
        props
            .iter()
            .find(|p| p.key == key)
            .unwrap_or_else(|| panic!("{key} not parsed"))
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn wiki(key: &str) -> WikiProp {
        WikiProp {
            key: key.into(),
            kind: Kind::Bool,
            help: format!("wiki help for {key}"),
            choices: vec![],
            multi: false,
        }
    }

    fn cache(window: Vec<WikiProp>) -> DocsCache {
        DocsCache {
            version: "0.99.0".into(),
            fetched_at: "2026-01-02 03:04".into(),
            source: "test".into(),
            window,
            workspace: vec![],
            layer: vec![],
        }
    }

    #[test]
    fn parses_window_effects() {
        let props = parse_effects(WINDOW);
        assert!(props.len() >= 55, "{}", props.len());
        let idle = find(&props, "idle_inhibit");
        assert_eq!(
            idle.choices,
            strs(&["none", "always", "focus", "fullscreen"])
        );
        assert!(!idle.multi);
        assert_eq!(idle.kind, Kind::Str);
        assert!(idle.help.starts_with("Sets an idle inhibit rule"));
        let suppress = find(&props, "suppress_event");
        assert!(suppress.multi);
        assert_eq!(suppress.choices.len(), 6);
        assert_eq!(suppress.choices[5], "x11configurerequest");
        assert_eq!(
            find(&props, "content").choices,
            strs(&["none", "photo", "video", "game"])
        );
        assert_eq!(
            find(&props, "tonemap").choices,
            strs(&["on", "off", "clamp", "limited"])
        );
        assert_eq!(find(&props, "float").kind, Kind::Bool);
        assert!(find(&props, "float").choices.is_empty());
        assert_eq!(find(&props, "border_size").kind, Kind::Int);
        assert_eq!(find(&props, "rounding_power").kind, Kind::Float);
        // free-text strings don't get choices from their examples
        assert!(find(&props, "animation").choices.is_empty());
        assert!(find(&props, "opacity").choices.is_empty());
        assert!(find(&props, "workspace").choices.is_empty());
        // Links and backticks are gone from the help text
        assert!(!find(&props, "move").help.contains(['[', '`']));
    }

    #[test]
    fn skips_match_props_and_comments() {
        let props = parse_effects(WINDOW);
        // `class` only exists in the match-prop table; `initial_title` too
        assert!(
            props
                .iter()
                .all(|p| p.key != "class" && p.key != "initial_title")
        );
        // rows after the `<!-- SORT … -->` comment are still read
        find(&props, "allows_input");
        find(&props, "force_rgbx");
        let md = "<!-- multi\nline | comment -->\n| Effect | Description | Type |\n| --- | --- | --- |\n| a | A <!-- hidden --> thing | bool |\n";
        let props = parse_effects(md);
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].help, "A thing");
    }

    #[test]
    fn parses_layer_and_workspace_pages() {
        let layer = parse_effects(LAYER);
        assert_eq!(layer.len(), 10);
        assert!(layer.iter().all(|p| p.key != "namespace"));
        assert_eq!(find(&layer, "ignore_alpha").kind, Kind::Float);
        let ws = parse_effects(WORKSPACE);
        assert_eq!(ws.len(), 15);
        assert_eq!(find(&ws, "float_gaps").kind, Kind::Str);
        assert_eq!(find(&ws, "persistent").kind, Kind::Bool);
    }

    #[test]
    fn unparseable_rows_still_give_name_and_help() {
        let md = "| Effect | Description | Type |\n| --- | --- | --- |\n| weird | Does `things` somehow | thing-ish |\n| bad row |\n";
        let props = parse_effects(md);
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].kind, Kind::Str);
        assert_eq!(props[0].help, "Does things somehow");
        assert!(props[0].choices.is_empty());
    }

    #[test]
    fn merge_overrides_and_adds() {
        let mut idle = wiki("idle_inhibit");
        idle.kind = Kind::Str;
        idle.choices = strs(&["a", "b"]);
        let mut suppress = wiki("suppress_event");
        suppress.choices = strs(&["x", "y"]);
        suppress.multi = true;
        let fresh = wiki("brand_new");
        // wiki row without choices keeps the built-in choices
        let mut content = wiki("content");
        content.help = "new content help".into();
        let c = PropCatalog::merged(Some(&cache(vec![idle, suppress, fresh, content])));
        let idle = c.info(RuleKind::Window, "idle_inhibit").unwrap();
        assert_eq!(idle.choices, strs(&["a", "b"]));
        assert_eq!(idle.help, "wiki help for idle_inhibit");
        let sup = c.info(RuleKind::Window, "suppress_event").unwrap();
        assert!(sup.multi);
        assert_eq!(sup.choices, strs(&["x", "y"]));
        let content = c.info(RuleKind::Window, "content").unwrap();
        assert_eq!(content.help, "new content help");
        assert_eq!(content.choices.len(), 4);
        // unknown wiki keys appear, with the wiki's type
        let new = c.info(RuleKind::Window, "brand_new").unwrap();
        assert_eq!(new.kind, Kind::Bool);
        assert!(!new.removed);
        // built-in keys stay
        assert!(c.info(RuleKind::Window, "float").is_some());
        assert!(c.status().contains("0.99.0") && c.status().contains("2026-01-02"));
        assert_eq!(PropCatalog::builtin().status(), "built-in list");
    }

    #[test]
    fn builtin_keys_missing_from_a_full_table_are_removed() {
        let keys: Vec<&str> = model::props_for(RuleKind::Window)
            .iter()
            .take(12)
            .map(|p| p.key)
            .collect();
        let c = PropCatalog::merged(Some(&cache(keys.iter().map(|k| wiki(k)).collect())));
        let last = model::props_for(RuleKind::Window).last().unwrap().key;
        assert!(c.info(RuleKind::Window, last).unwrap().removed);
        assert!(!c.info(RuleKind::Window, keys[0]).unwrap().removed);
        assert!(c.removed_note().contains("0.99.0"));
        // other kinds had no wiki rows: nothing removed there
        assert!(c.props(RuleKind::Layer).iter().all(|p| !p.removed));
        assert!(c.props(RuleKind::Workspace).iter().all(|p| !p.removed));
    }

    #[test]
    fn too_few_rows_remove_nothing() {
        let c = PropCatalog::merged(Some(&cache(vec![wiki("float"), wiki("tile")])));
        assert!(c.props(RuleKind::Window).iter().all(|p| !p.removed));
        assert!(
            PropCatalog::builtin()
                .props(RuleKind::Window)
                .iter()
                .all(|p| !p.removed)
        );
    }

    #[test]
    fn the_real_pages_remove_few_builtin_keys() {
        let full = DocsCache {
            window: parse_effects(WINDOW),
            workspace: parse_effects(WORKSPACE),
            layer: parse_effects(LAYER),
            ..cache(vec![])
        };
        let c = PropCatalog::merged(Some(&full));
        let gone: Vec<&str> = RuleKind::ALL
            .iter()
            .flat_map(|k| c.props(*k))
            .filter(|p| p.removed)
            .map(|p| p.key.as_str())
            .collect();
        // the workspace selector is exempt; only on_created_empty isn't on the wiki
        assert_eq!(gone, ["on_created_empty"]);
        assert!(c.info(RuleKind::Workspace, "workspace").is_some());
    }

    #[test]
    fn cache_round_trips_in_a_temp_dir() {
        let dir = std::env::temp_dir().join(format!("hypr-rules-docs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(load_from(&dir, "0.99.0").is_none());
        let mut c = cache(parse_effects(WINDOW));
        c.layer = parse_effects(LAYER);
        save_to(&dir, &c).unwrap();
        assert_eq!(load_from(&dir, "0.99.0"), Some(c));
        // another version has its own file
        assert!(load_from(&dir, "1.0.0").is_none());
        // a damaged file is ignored
        fs::write(cache_file(&dir, "bad"), "{ nope").unwrap();
        assert!(load_from(&dir, "bad").is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cache_file_names_are_safe() {
        let p = cache_file(Path::new("/x"), "../0.5 rc/1");
        assert_eq!(p, Path::new("/x/props-.._0.5_rc_1.json"));
    }
}
