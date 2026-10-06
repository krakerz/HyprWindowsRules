# Changelog

## [Unreleased]

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
