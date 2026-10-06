# Hyprland Windows Rules

![Hyprland Windows Rules screenshot](https://gist.githubusercontent.com/krakerz/97ccc96ce96ef1bb63d03ef9fc0186e9/raw/screenshot.png)

A GUI application for managing Hyprland window rules without editing config files.

## Description

Hyprland 0.55+ moved to a Lua config; window rules are `hl.window_rule({ match = {...}, ... })` calls. Hand-writing them means regex for class/title matching and editing config files. hypr-rules is a small GUI app (built with iced) that keeps all window rules in one managed Lua file and edits them through a form. The interface follows the system light/dark mode by default and can be switched to light or dark. Hyprland matches rule patterns against the whole class/title (full match), and the app's simple modes translate plain text into the right pattern. Lua 5.4 is embedded in the binary.

## Features

- Manage window rules through a graphical interface without editing config files.
- Match windows by class or title using simple text patterns: exactly, starts with, contains, ends with, or raw regex.
- Support case-insensitive matching with an "any case" toggle.
- Preview which open windows a rule matches in real time.
- Create rules from selected open windows with optional class, title, size, position, and workspace.
- Set common properties: floating/tiled mode, size, position, center on screen, workspace with silent/monitor/pin options, and initial focus, with workspaces and monitors pickable from a drop-down.
- Edit any additional window-rule property via a property table.
- Import existing rules from any Hyprland `.lua` file, optionally commenting out originals with a backup.
- Persist rules to `~/.config/hypr-rules/hypr-rules.lua` (or a custom path) with automatic timestamped backups.
- Reload Hyprland on save and display config errors for debugging.
- Set up or verify the one-click hook in your Hyprland config to load the rules file automatically.
- Light, dark, or follow-the-system theme, switchable from the toolbar.
- Updates itself from GitHub releases: a notice at start-up, then one click to download, install and restart.

## Installation

Requires Hyprland 0.55+ with the Lua config — nothing else at runtime. Download from the [Releases page](https://github.com/krakerz/HyprWindowsRules/releases).

**Option A: Tar.gz archive**
Extract the downloaded `hypr-rules-<version>-x86_64.tar.gz` and run:
```bash
./install.sh
```
This installs the binary to `~/.local/bin/hypr-rules`, adds a desktop entry to `~/.local/share/applications/` (so the app appears in your launcher), and installs the icon. To uninstall, run `./uninstall.sh` from the extracted directory; it removes the binary, desktop entry, and icon while leaving your rules file and Hyprland hook intact.

**Option B: AppImage**
Download `hypr-rules-<version>-x86_64.AppImage`, make it executable, and run it:
```bash
chmod +x hypr-rules-*-x86_64.AppImage
./hypr-rules-*-x86_64.AppImage
```
The AppImage is standalone and requires no installation.

## Building from source

Prerequisites: Rust 1.90+ (cargo) and a C compiler (the embedded Lua is compiled from source).

```bash
git clone https://github.com/krakerz/HyprWindowsRules.git
cd HyprWindowsRules
cargo build --release
```

The result is a single standalone binary at `target/release/hypr-rules`.

To build and install in one step from a clone:
```bash
packaging/install.sh
```
This builds with cargo and then installs the binary, desktop entry, and icon to the same locations as the tar.gz archive's install script.

## Usage

**Quick start:** Launch the app, click "From window…", pick a window, choose Floating + size, then Save — Hyprland reloads and the rule applies to newly opened windows.

**First launch:** The app offers to import rules already in your config.

**Settings:** Configure the rules file path, which file to load it from (defaults to `~/.config/hypr/hyprland.lua`), and toggle reload-on-save behavior.

**The hook:** By default, hypr-rules adds these lines to the end of `~/.config/hypr/hyprland.lua`, in the same style Caelestia uses to load `hypr-user.lua`:
```lua
-- Window rules managed by the hypr-rules app
package.path = package.path .. ";" .. home .. "/.config/hypr-rules/?.lua" -- hypr-rules
maybe_create(home .. "/.config/hypr-rules/hypr-rules.lua") -- hypr-rules
require("hypr-rules") -- hypr-rules
```

Because the file is loaded with `require`, Hyprland watches it and reloads by itself whenever you save. In a config without Caelestia's `home` and `maybe_create` helpers, the app writes a single self-contained line instead that does the same thing. Installing the hook also removes the old hook line from Caelestia's `~/.config/caelestia/hypr-user.lua` if one is there.

**Rule priority:** Later rules win when two rules set the same property; use Move up/down to reorder them.

**Editing by hand:** The managed file stays plain Lua and can be hand-edited as long as it remains a list of `hl.window_rule` calls; comments are not preserved when the app saves.

**Updates:** The app checks for a new release at start-up and shows a notice in the status bar if one is available. Click to download and install automatically with a progress bar, then restart. Settings → Updates shows your current version and install method, with a "Check now" button and a "Check for updates at startup" toggle (on by default). Builds run from source (cargo run or the binary in target/) don't update themselves.

## FAQ

**Why doesn't my rule apply to a window that's already open?**
Most window rules apply when a window opens; reopen the window to test the rule.

**Why does my rule not match?**
Matching is case-sensitive and covers the whole class — use the live preview, enable "Any case", or switch to "contains" mode.

**Will it touch my other Hyprland config?**
Only the one hook line in the file you choose (by default `~/.config/hypr/hyprland.lua`); imported originals are commented out only if you tick the box, with a backup created. Installing also removes the old hook line from Caelestia's `~/.config/caelestia/hypr-user.lua` if one is there.

**The "Load rules in Hyprland" button came back — why?**
The hook line is gone from the "Load it from" file, e.g. because a Caelestia update replaced `~/.config/hypr/hyprland.lua`. Click the button to add it again.

**Can I still edit hypr-rules.lua by hand?**
Yes, as long as it remains a list of `hl.window_rule` calls; comments are not preserved when the app saves.

---

### Notes

Built and maintained with the help of AI.
