//! Per-program audio routes, applied by a WirePlumber script.
//!
//! The app owns two WirePlumber files: the script and a .conf that loads it and
//! holds the routes as JSON (`hypr-rules.audio-routes`). WirePlumber scripts can't
//! read files, so a running WirePlumber gets route changes through the
//! "hypr-rules-audio" metadata instead; the .conf is what it starts with.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

const SCRIPT: &str = include_str!("../assets/wireplumber/hypr-rules-audio.lua");
const METADATA: &str = "hypr-rules-audio";
const SECTION: &str = "hypr-rules.audio-routes = ";

/// The .conf up to the routes section; the routes JSON is appended after it.
const CONF_HEAD: &str = "\
# Managed by Hyprland Windows Rules — per-program audio routes.
# Registers hypr-rules-audio.lua; the routes below are its starting point and
# can be changed live through the \"hypr-rules-audio\" PipeWire metadata.
wireplumber.components = [
  {
    name = hypr-rules-audio.lua, type = script/lua,
    provides = script.hypr-rules-audio
    requires = [ policy.linking.standard ]
  }
  # Required by the profile but only *wants* the script: a broken script can
  # never stop WirePlumber from starting (an \"optional\" feature isn't loaded
  # unless something pulls it in).
  {
    type = virtual, provides = hypr-rules.audio
    wants = [ script.hypr-rules-audio ]
  }
]
wireplumber.profiles = {
  main = {
    hypr-rules.audio = required
  }
}
";

fn script_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| crate::settings::home().join(".local/share"))
        .join("wireplumber/scripts/hypr-rules-audio.lua")
}

fn conf_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| crate::settings::home().join(".config"))
        .join("wireplumber/wireplumber.conf.d/60-hypr-rules-audio.conf")
}

/// How a route recognizes a device. Any field that's set and matches is enough:
/// the description survives a device changing its node name (the Maxwell does,
/// between dongle and USB), the node name survives a renamed description.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Device {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// node.name glob, `*` matching anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

impl Device {
    pub fn label(&self) -> String {
        self.description
            .clone()
            .or_else(|| self.name.clone())
            .or_else(|| self.pattern.clone())
            .unwrap_or_default()
    }
}

fn is_true(b: &bool) -> bool {
    *b
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    #[serde(default)]
    pub name: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub programs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Device>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Device>,
    /// Extra stream-property matches, kept as written (not edited in the app).
    #[serde(default, rename = "match", skip_serializing_if = "Option::is_none")]
    pub extra_match: Option<Json>,
}

impl Default for Route {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            programs: Vec::new(),
            output: None,
            input: None,
            extra_match: None,
        }
    }
}

impl Route {
    pub fn title(&self) -> String {
        if !self.name.is_empty() {
            self.name.clone()
        } else if !self.programs.is_empty() {
            self.programs.join(", ")
        } else {
            "(new route)".into()
        }
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(d) = &self.output {
            parts.push(format!("output → {}", d.label()));
        }
        if let Some(d) = &self.input {
            parts.push(format!("input → {}", d.label()));
        }
        if parts.is_empty() {
            "no devices".into()
        } else {
            parts.join(", ")
        }
    }
}

/// Whether the script and .conf are in place, and whether WirePlumber runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Never set up: the first save installs the script.
    NotInstalled,
    /// Turned off: routes are kept, but WirePlumber's files are removed.
    Off,
    /// Installed (or updated) but WirePlumber hasn't loaded the current script yet.
    NeedsRestart,
    Active,
}

/// The app's own copy of the routes — kept when routing is turned off, unlike the
/// WirePlumber .conf.
fn routes_path() -> PathBuf {
    crate::settings::config_dir().join("audio-routes.json")
}

pub fn load_routes() -> Result<Vec<Route>, String> {
    match fs::read_to_string(routes_path()) {
        Ok(t) => {
            return serde_json::from_str(&t)
                .map_err(|e| format!("couldn't read {}: {e}", routes_path().display()));
        }
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.to_string()),
        Err(_) => {}
    }
    // Before audio-routes.json existed, the routes lived only in the .conf
    let text = match fs::read_to_string(conf_path()) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let Some(i) = text.find(SECTION) else {
        return Ok(Vec::new());
    };
    serde_json::from_str(&text[i + SECTION.len()..])
        .map_err(|e| format!("couldn't read the routes in {}: {e}", conf_path().display()))
}

/// Saves the routes without touching WirePlumber (used while routing is off).
pub fn save_routes(routes: &[Route]) -> io::Result<()> {
    write(&routes_path(), &format!("{}\n", routes_json(routes)))
}

fn routes_json(routes: &[Route]) -> String {
    serde_json::to_string_pretty(routes).unwrap_or_else(|_| "[]".into())
}

/// Writes the script and .conf. Returns true when the script changed, i.e.
/// WirePlumber must restart to run the new code.
pub fn install(routes: &[Route]) -> io::Result<bool> {
    save_routes(routes)?;
    let script = script_path();
    let script_changed = fs::read_to_string(&script).ok().as_deref() != Some(SCRIPT);
    if script_changed {
        write(&script, SCRIPT)?;
    }
    write(
        &conf_path(),
        &format!("{CONF_HEAD}{SECTION}{}\n", routes_json(routes)),
    )?;
    Ok(script_changed)
}

fn write(path: &PathBuf, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

pub fn uninstall() -> io::Result<()> {
    for p in [conf_path(), script_path()] {
        match fs::remove_file(&p) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// Hands the routes to the running script, which re-places streams right away.
pub fn apply_live(routes: &[Route]) -> Result<(), String> {
    let out = Command::new("pw-metadata")
        .args([
            "-n",
            METADATA,
            "0",
            "routes",
            &routes_json(routes),
            "Spa:String:JSON",
        ])
        .output()
        .map_err(|e| format!("couldn't run pw-metadata: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub fn restart_wireplumber() -> Result<(), String> {
    let out = Command::new("systemctl")
        .args(["--user", "restart", "wireplumber"])
        .output()
        .map_err(|e| format!("couldn't run systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// A sink or source as the device pickers show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub description: String,
    pub name: String,
}

impl std::fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.description)
    }
}

/// What PipeWire currently has: devices, the programs playing or recording, and
/// whether our script's metadata exists (i.e. WirePlumber runs the script).
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub outputs: Vec<DeviceInfo>,
    pub inputs: Vec<DeviceInfo>,
    pub programs: Vec<String>,
    script_running: bool,
}

/// Mirrors program_of in the script: the binary, or the name in PipeWire's ALSA
/// plugin's "alsa_playback.<program>" node name.
fn program_of(props: &Json) -> Option<String> {
    let s = |k: &str| props.get(k).and_then(Json::as_str);
    if let Some(b) = s("application.process.binary") {
        return Some(b.to_string());
    }
    let name = s("node.name").unwrap_or("");
    name.strip_prefix("alsa_playback.")
        .or_else(|| name.strip_prefix("alsa_capture."))
        .or_else(|| {
            s("application.name")?
                .strip_prefix("PipeWire ALSA [")?
                .strip_suffix(']')
        })
        .map(String::from)
}

pub fn snapshot() -> Result<Snapshot, String> {
    let out = Command::new("pw-dump")
        .output()
        .map_err(|e| format!("couldn't run pw-dump: {e}"))?;
    let objects: Vec<Json> = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("unexpected pw-dump output: {e}"))?;
    let mut snap = Snapshot::default();
    for o in &objects {
        let props = &o["info"]["props"];
        match o["type"].as_str() {
            Some("PipeWire:Interface:Metadata") => {
                // Metadata objects keep their props at the top level, not under "info"
                if o["props"]["metadata.name"].as_str() == Some(METADATA) {
                    snap.script_running = true;
                }
            }
            Some("PipeWire:Interface:Node") => {
                let class = props["media.class"].as_str().unwrap_or("");
                let device = || DeviceInfo {
                    description: props["node.description"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    name: props["node.name"].as_str().unwrap_or_default().to_string(),
                };
                match class {
                    "Audio/Sink" => snap.outputs.push(device()),
                    "Audio/Source" => snap.inputs.push(device()),
                    "Stream/Output/Audio" | "Stream/Input/Audio" => {
                        snap.programs.extend(program_of(props))
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    snap.programs.sort();
    snap.programs.dedup();
    Ok(snap)
}

pub fn status(snap: &Snapshot) -> Status {
    let installed = fs::read_to_string(script_path()).ok();
    match installed {
        Some(_) if !conf_path().exists() => Status::NotInstalled,
        Some(s) if s != SCRIPT || !snap.script_running => Status::NeedsRestart,
        Some(_) => Status::Active,
        None if routes_path().exists() => Status::Off,
        None => Status::NotInstalled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_round_trip_and_keep_extra_matches() {
        let json = r#"[{"name":"Chat apps","programs":["Discord","mumble"],
            "output":{"description":"Maxwell Chat","pattern":"alsa_output.usb-*-01.pro-output-0"},
            "match":{"media.role":"Communication"}},{"programs":["x"],"enabled":false}]"#;
        let routes: Vec<Route> = serde_json::from_str(json).unwrap();
        assert_eq!(routes[0].output.as_ref().unwrap().label(), "Maxwell Chat");
        assert!(routes[0].enabled && !routes[1].enabled);
        let again: Vec<Route> = serde_json::from_str(&routes_json(&routes)).unwrap();
        assert_eq!(again, routes);
        assert!(routes_json(&routes).contains("\"match\""));
    }

    #[test]
    fn program_names_from_pulse_and_alsa_streams() {
        let p = |v: Json| program_of(&v);
        assert_eq!(
            p(serde_json::json!({"application.process.binary": "Discord"})).as_deref(),
            Some("Discord")
        );
        assert_eq!(
            p(serde_json::json!({"node.name": "alsa_playback.mumble"})).as_deref(),
            Some("mumble")
        );
        assert_eq!(
            p(serde_json::json!({"application.name": "PipeWire ALSA [mumble]"})).as_deref(),
            Some("mumble")
        );
        assert_eq!(p(serde_json::json!({"node.name": "Cider"})), None);
    }

    #[test]
    fn generated_conf_reads_back() {
        let routes = vec![Route {
            programs: vec!["mumble".into()],
            ..Default::default()
        }];
        let text = format!("{CONF_HEAD}{SECTION}{}\n", routes_json(&routes));
        let i = text.find(SECTION).unwrap();
        let back: Vec<Route> = serde_json::from_str(&text[i + SECTION.len()..]).unwrap();
        assert_eq!(back, routes);
    }
}
