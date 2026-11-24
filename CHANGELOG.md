# Changelog

## [Unreleased]

## [0.7.0] — 2026-11-24

### Added
- Window inspector showing which rules hit a window and the final result.
- Rule checks for overlapping, overridden and never-matching rules.
- "Apply now" applies a rule to open windows without reopening them.
- Pick a window by focusing it (3-second countdown).
- History of saved versions with a diff and one-click restore.
- Undo and redo (Ctrl+Z, Ctrl+Shift+Z / Ctrl+Y).
- Rule groups that collapse and switch on or off together.
- Workspace rules and layer rules.
- Typed property editors with descriptions, and a warning for unknown properties.
- Match-count badge on each rule.
- Rules are checked before saving.
- Ctrl+F jumps to the filter.
- Drag rules to reorder them.
- Right-click a rule for its actions.

### Changed
- Redesigned layout: aligned forms, consistent sizes and spacing, and a right-click menu on rules.
- The theme choice moved into Settings.
- Clearer button styles, with a distinct disabled look.
- Window rules and Audio routes are real tabs; rule actions moved next to the list.
- The form keeps a readable width and uses two columns on wide windows.
- Unsaved rules are marked in the list and on the Save button.
- The status bar shows paths with `~`.
- Selected rules' details stay readable.

### Fixed
- Editing a rule no longer drops effects the editor can't show (e.g. `size = "1000 700"`) or garbles table values under Other properties.
- "Save here" after changing the rules file in Settings now saves.
- "Load rules in Hyprland" no longer creates a Hyprland config that doesn't exist; it says so instead.
- The app no longer crashes at start-up on a config file with a BOM or other non-ASCII text outside strings.
- Import only offers to comment out rules that sit on their own lines, so other code on those lines is never commented out.
- Saving through a symlinked config or rules file keeps the symlink.
- Importing from a config that loads the managed rules file no longer lists those rules again.
- Names, titles and patterns with control characters are saved as valid Lua.
- Settings refuses a rules file name Hyprland's `require` can't load.
- Duplicate rule names are always made unique.
- The updater no longer mistakes another AppImage (e.g. the editor a terminal was opened from) for this app's own.
- Turning audio routing off clears the unsaved-changes mark.

## [0.6.0] — 2026-10-13

### Added
- Audio routes: a drop-down of each device's hardware name, so a device can be matched across connection modes without typing its name.
- Audio routes: a notice when a route's device isn't connected, saying the programs use the default device until it returns.

## [0.5.0] — 2026-10-07

### Added
- Several conditions on the same field now mean "any of these", e.g. title starts with Extracting or Copy.
- "Copy from window…" buttons next to Size and Position fill them from an open window.

### Fixed
- A second condition on the same field no longer silently replaces the first when saving.

## [0.4.0] — 2026-10-07

### Changed
- Only the 10 newest backups of the rules file are kept.

## [0.3.0] — 2026-10-06

### Added
- Built-in updater: checks for a new release at start-up and installs it with one click.
- All window-rule effects Hyprland 0.56 supports are offered under "Other properties" (16 more, e.g. `no_follow_mouse`, `border_color`, `confine_pointer`).
- Audio routes: send a program's sound and microphone to a chosen device, e.g. chat apps to a headset's chat channel.

### Changed
- The app is now called Hyprland Windows Rules.

### Fixed
- Turning audio routing off no longer deletes your audio routes.

## [0.2.0] — 2026-10-06

### Added
- Workspace drop-down listing existing workspaces (special ones included) and those used by other rules; free text still works for new ones.
- Theme picker in the toolbar: system, light or dark.
- Release downloads: a tar.gz with an install script that puts the app in `~/.local/bin` and adds it to the app launcher, and a standalone AppImage.
- App icon.

### Changed
- Rewritten in Rust as a single standalone binary; Python, PySide6 and a system Lua are no longer needed.
- Rules are now loaded from `~/.config/hypr/hyprland.lua` by default, replacing the old line in Caelestia's `hypr-user.lua`.
- The interface no longer uses the Qt theme; it follows the system light/dark mode by default.
- The hook follows Caelestia's style (`require` with its `home` and `maybe_create` helpers) when installed into Caelestia's `hyprland.lua`.
- The managed rules file is renamed to `~/.config/hypr-rules/hypr-rules.lua`; an existing `rules.lua` is moved over automatically.

### Fixed
- Saved rules not always taking effect until Hyprland was reloaded by hand.

## [0.1.0] — 2026-10-06

### Added
- Manage window rules through a graphical interface without editing config files.
- Match windows by class or title using simple text patterns: exactly, starts with, contains, ends with, or raw regex.
- Support case-insensitive matching with an "any case" toggle.
- Preview which open windows a rule matches in real time.
- Create rules from selected open windows with optional class, title, size, position, and workspace.
- Set common properties: floating/tiled mode, size, position, center on screen, workspace with silent/monitor/pin options, and initial focus.
- Edit any additional window-rule property via a property table.
- Import existing rules from any Hyprland `.lua` file, optionally commenting out originals with a backup.
- Persist rules to `~/.config/hypr-rules/rules.lua` with configurable path and automatic timestamped backups.
- Reload Hyprland on save and display config errors for debugging.
- Set up or verify the one-click hook in your Hyprland config to load the rules file automatically.
