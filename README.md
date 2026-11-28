# Hyprland Windows Rules

A GUI application for managing Hyprland window, workspace, and layer rules without editing config files.

![Hyprland Windows Rules screenshot](https://gist.githubusercontent.com/krakerz/97ccc96ce96ef1bb63d03ef9fc0186e9/raw/screenshot.png)

## Description

Hyprland 0.55+ uses a Lua config where rules are `hl.window_rule`, `hl.workspace_rule`, and `hl.layer_rule` calls. Hand-writing them means writing regex patterns for class/title matching and editing config files by hand. `hypr-rules` is a small GUI app (built with `iced`) that keeps your rules in one managed Lua file and edits them through a form, using `hyprctl` to inspect open windows, monitors, and workspaces and to reload Hyprland on save. Hyprland matches rule patterns against the whole class or title (full match), and the app's simple match modes translate plain text into the right pattern. Lua 5.4 is embedded in the binary.

## Features

- Manage window, workspace, and layer rules through a graphical interface without editing config files.
- Match windows by class or title using simple text patterns: exactly, starts with, contains, ends with, or raw regex, with an "any case" toggle for case-insensitive matching.
- Preview which open windows a rule matches in real time, with match-count badges.
- Create rules from selected open windows (or pick a window by focusing it) with optional class, title, size, position, and workspace.
- Set common properties (floating/tiled mode, size, position, center on screen, workspace with silent/monitor/pin options, and initial focus) with workspaces and monitors pickable from drop-downs.
- Edit additional rule properties with typed editors, fixed-value drop-downs/toggles, and descriptions cached from the Hyprland wiki for your installed Hyprland version.
- Inspect windows to see which rules match and the final effective result, with checks for overridden, overlapping, and never-matching rules.
- Apply a rule immediately to already-open windows with "Apply now".
- Group rules into collapsible groups that switch on or off together, drag rules to reorder them, and use a right-click context menu for rule actions.
- Undo/redo edits, review pre-save checks, and browse saved version history with diffs and one-click restore.
- Import existing rules from any Hyprland `.lua` file, optionally commenting out originals with a backup.
- Persist rules to `~/.config/hypr-rules/hypr-rules.lua` (or a custom path) with automatic timestamped backups (keeping the 10 newest).
- Reload Hyprland on save and display config errors for debugging, with a one-click hook in your Hyprland config to load the rules file automatically.
- Choose system, light, or dark theme in Settings.
- Route per-program audio playback and microphone streams to chosen devices (e.g., chat apps to a headset's chat channel), applied live via a WirePlumber script.
- Self-update from GitHub releases with a start-up notice and one-click download, install, and restart.

## Dependencies

### Runtime

- **Hyprland ≥ 0.55** with Lua configuration (`hyprctl` and `Hyprland` on `PATH`).
- **Wayland / X11 and GPU system libraries** (loaded dynamically at runtime via `dlopen` by `iced`).
- **PipeWire and WirePlumber 0.5** (`pw-dump`, `pw-metadata`, and `systemctl --user` to restart `wireplumber`) — optional, only needed for the Audio routes tab.
- **XDG Desktop Portal** (`xdg-desktop-portal` implementation) — optional, for native file chooser dialogs.
- **`tar` and `bash`** — needed if using the in-app self-updater with a `.tar.gz` release archive (`install.sh` also uses `update-desktop-database` and `gtk-update-icon-cache` when available).

### Build

- **Rust ≥ 1.90** (`cargo`; `rustfmt` and `clippy` for formatting/lint checks).
- **A C compiler (`cc`)** — compiles the vendored Lua 5.4 source (`mlua` `lua54` + `vendored` features). No system Lua or Wayland/X11 `-dev` packages are required.

## Installation

Download the latest release from the [Releases page](https://github.com/krakerz/HyprWindowsRules/releases).

**Option A: Tar.gz archive**

Extract `hypr-rules-<version>-x86_64.tar.gz` and run:

```bash
./install.sh
```

This installs the binary to `~/.local/bin/hypr-rules`, adds a desktop entry to `~/.local/share/applications/hypr-rules.desktop`, and installs the icon to `~/.local/share/icons/hicolor/scalable/apps/hypr-rules.svg`.

To uninstall, run `./uninstall.sh` from the extracted directory; it removes the binary, desktop entry, and icon while leaving your rules file, Hyprland hook, and any WirePlumber audio routes in place.

**Option B: AppImage**

Download `hypr-rules-<version>-x86_64.AppImage`, make it executable, and run it:

```bash
chmod +x hypr-rules-*-x86_64.AppImage
./hypr-rules-*-x86_64.AppImage
```

The AppImage is standalone and requires no installation.

## Building from source

See [Dependencies](#dependencies) for the required Rust toolchain and C compiler. From a clean clone:

```bash
git clone https://github.com/krakerz/HyprWindowsRules.git
cd HyprWindowsRules
cargo build --release --locked
```

This produces a standalone binary at `target/release/hypr-rules`.

To build and install into `~/.local/bin` (along with the desktop entry and icon) in one step:

```bash
./packaging/install.sh
```

## Usage

**Quick start:** Launch `hypr-rules`, click "From window…", pick a window, enable Floating and set a size, then click Save — Hyprland reloads and the rule applies to newly opened windows.

**First launch:** The app offers to import rules already present in your Hyprland config.

**Settings:** Configure the managed rules file path, which file loads it (defaults to `~/.config/hypr/hyprland.lua`), whether to reload Hyprland on save, your theme preference (System, Light, or Dark), and update checking.

**The hook:** By default, `hypr-rules` adds these lines to the end of `~/.config/hypr/hyprland.lua`, matching Caelestia's `hypr-user.lua` loader style:

```lua
-- Window rules managed by the hypr-rules app
package.path = package.path .. ";" .. home .. "/.config/hypr-rules/?.lua" -- hypr-rules
maybe_create(home .. "/.config/hypr-rules/hypr-rules.lua") -- hypr-rules
require("hypr-rules") -- hypr-rules
```

Because the file is loaded with `require`, Hyprland watches it and reloads automatically whenever you save. In a config without Caelestia's `home` and `maybe_create` helpers, the app writes a single self-contained line instead. Installing the hook also removes the old hook line from Caelestia's `~/.config/caelestia/hypr-user.lua` if present.

**Rule priority:** Later rules win when two rules set the same property; drag rules in the list (or press `Alt+Up`/`Alt+Down`) to reorder them.

**Shortcuts:** `Ctrl+S` save, `Ctrl+N` new rule, `Ctrl+W` new rule from a window, `Ctrl+D` duplicate, `Delete` remove, `Alt+Up`/`Alt+Down` move, `Ctrl+Z` undo, `Ctrl+Shift+Z` or `Ctrl+Y` redo, `Ctrl+F` filter, `F5` refresh windows.

**Editing by hand:** The managed file stays plain Lua and can be hand-edited as long as it remains a list of `hl.window_rule` / `hl.workspace_rule` / `hl.layer_rule` calls; the app keeps its own `-- @group` lines but drops other comments when saving.

**Updates:** The app checks for a new release at start-up and shows a notice in the status bar if one is available, with one-click download, install, and restart. **Settings → Updates** shows your current version and install method, with a "Check now" button and a "Check for updates at startup" toggle (on by default). Source builds (`cargo run` or running from `target/`) do not self-update.

**Audio routes:** The Audio routes tab sends a program's playback (output) and microphone (input) streams to a chosen device (for example, Discord and Mumble to a headset's chat channel while games use the default device). Pick programs by executable name (listed while playing or recording, or entered manually) — matched across both PulseAudio and ALSA clients. Match devices by description, node name, or a `*` wildcard pattern for devices whose internal node name changes across connection modes. On first save, the app installs a WirePlumber script at `~/.local/share/wireplumber/scripts/hypr-rules-audio.lua` and config at `~/.config/wireplumber/wireplumber.conf.d/60-hypr-rules-audio.conf`, and WirePlumber requires one restart to load them. Thereafter, changes apply immediately via `pw-metadata` and continue working when the app is closed. Routes override a stream's own device choice or manual mixer moves. Clicking Turn off removes both WirePlumber files and stops routing while keeping your saved routes; other WirePlumber config is untouched. Tip: leave the program's own audio output/input setting on "Default".

## FAQ

**Why doesn't my rule apply to a window that's already open?**
Most window rules apply when a window opens; reopen the window to test the rule, or use "Apply now" to apply supported effects to open windows.

**Why does my rule not match?**
Matching is case-sensitive and covers the whole class or title — use the live preview, enable "Any case", or switch to "contains" mode.

**Will it touch my other Hyprland config?**
Only the hook lines in the file you choose (by default `~/.config/hypr/hyprland.lua`); imported originals are commented out only if you tick the box, with a backup created. Installing the hook also removes the old hook line from Caelestia's `~/.config/caelestia/hypr-user.lua` if present.

**The "Load rules in Hyprland" button came back — why?**
The hook line is missing from the "Load it from" file (for example, after a Caelestia update replaced `~/.config/hypr/hyprland.lua`). Click the button to add it again.

**Can I still edit `hypr-rules.lua` by hand?**
Yes, as long as it remains a list of `hl.window_rule` / `hl.workspace_rule` / `hl.layer_rule` calls; the app keeps its own `-- @group` lines but drops other comments when saving.

**Does `uninstall.sh` remove the audio routing?**
No — like your window rules, audio routing keeps working without the app. Use Turn off in the Audio routes tab first if you want it removed.

## License

MIT License — see [LICENSE](LICENSE).

---

### Notes

Built and maintained with the help of AI.
