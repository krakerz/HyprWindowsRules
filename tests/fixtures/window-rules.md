---
weight: 10
title: Window Rules
---

## Syntax

```lua
hl.window_rule({
    name? = string,
    match = {
        prop = some_prop_value,
    },
    effect = some_effect_value,
})
```

## Window Rules

You can set window rules to achieve different window behaviors based on their properties.

### Props

The supported fields for the `match` table are:

| Prop | Description | Type |
| --- | --- | --- |
| class | Windows with `class` matching `RegEx` | [RegEx] |
| content | Windows with specified content type (none, photo, video, game) | string |
| focus | Currently focused window | bool |
| fullscreen | Fullscreen (covering or non-covering) windows | bool |
| fullscreen_state_client | Windows with matching `fullscreenstate`. `0` - none, `1` - maximize, `2` - fullscreen, `3` - maximize and fullscreen | int |
| fullscreen_state_internal | Windows with matching `fullscreenstate`. `0` - none, `1` - maximize, `2` - fullscreen, `3` - maximize and fullscreen | int |
| float | Floating windows | bool |
| group | Grouped windows | bool |
| initial_class | Windows with `initialClass` matching `RegEx` | [RegEx] |
| initial_title | Windows with `initialTitle` matching `RegEx` | [RegEx] |
| modal | Modal windows (e.g., "Are you sure" popups) | bool |
| pin | Pins the window (i.e. show it on all workspaces). **Important: pinning is ignored for non-floating windows** | bool |
| tag | Windows with matching `tag` | string |
| title | Windows with `title` matching `RegEx` | [RegEx] |
| workspace | Windows on matching workspace | workspace selector |
| xdg_tag | Match a window by its xdgTag | [RegEx] |
| xwayland | Xwayland windows | bool |

## Effects

### Static effects

> [!WARNING]
> It is not possible to `float` (or any other static rule) a window based on a change in the `title` after the window has been created.
> This applies to all static effects listed here.
>
> Instead, use a [dispatch](../../dispatchers) triggered by an [event](../../advanced-configuration/events) listener to apply the effect after the window has been created:
> ```lua
> hl.on("window.title", function(w)
>     if w ~= nil and w.title == "foo" then
>         hl.dispatch(hl.dsp.window.float({ action = "set" }))
>     end
> end)
> ```

Static effects are evaluated once when the window is opened and never again.
This essentially means that it is always the `initialTitle` and `initialClass` which will be found when matching on `title` and `class`, respectively.

| Effect | Description | Type |
| --- | --- | --- |
| center | If the window is floating, will center it on the monitor | bool |
| content | Sets content type: `"none"`, `"photo"`, `"video"`, or `"game"` | string |
| float | Floats a window | bool |
| fullscreen | Fullscreens a window | bool |
| fullscreen_state | Sets the fullscreen mode for the internal client (e.g., `"1 2"`). Values: `0` none, `1` maximize, `2` fullscreen, `3` maximize and fullscreen | string |
| group | Sets window group properties. See [group options](#group-window-rule-options) below | string |
| maximize | Maximizes a window | bool |
| monitor | Sets the monitor on which a window should open (e.g., `"1"`, `"DP-1"`). Can be suffixed with `" silent"` | string |
| move | Moves a floating window to the given monitor-local coordinates. See [position and size](#position-and-size) below | table or string |
| no_close_for | Makes the window uncloseable with `killactive` for a given number of ms on open | int |
| no_initial_focus | Disables the initial focus to the window | bool |
| pin | Pins the window (i.e. show it on all workspaces). _Note: pinning is ignored for non-floating windows. You most likely want to use this together with `float = true`_ | bool |
| pseudo | Pseudotiles a window | bool |
| scrolling_width | Set column width for window when starting on a workspace with the scrolling layout | float |
| size | Resizes a floating window. See [position and size](#position-and-size) below | table or string |
| suppress_event | Ignores specific events. Space-separated: `"fullscreen"`, `"maximize"`, `"activate"`, `"activatefocus"`, `"fullscreenoutput"`, `"x11configurerequest"` | string |
| tile | Tiles a window | bool |
| workspace | Sets the workspace on which a window should open. Can also be `"unset"` or suffixed with `" silent"` | string |

#### Position and size

<!-- TODO: Table arguments should be { x = int, y = int }, https://github.com/hyprwm/Hyprland/discussions/16339. When (I hope it is "when", and not "if") this is fixed, section below should be edited to mention the correct form -->

Position and size arguments take a Lua table or a string.
Value can be a number or an [expression](#expressions).
The string argument is split at the first whitespace.

A single value, a table without exactly two elements, and any separator other than a whitespace, e.g.:`"800,600"`, `"800x600"`, are config errors.

##### Expressions

Expressions are simple shortcodes for basic information.
All position variables are monitor-local.
To be correctly parsed in a string argument, they should not contain spaces.

- `monitor_w` and `monitor_h` for monitor size
- `window_x` and `window_y` for window position
- `window_w` and `window_h` for window size
- `cursor_x` and `cursor_y` for cursor position

{{% details title="Examples" closed="true" %}}

```lua
-- Set size to x = 800, y = 600
size = { 800, 600 }

-- Same size, but via string
size = "800 600"

-- Move window center to the cursor position
move = { "cursor_x - (window_w * 0.5)", "cursor_y - (window_h * 0.5)" }

-- Align window with the left monitor border and 17 pixels below monitor center
move = { "window_w * 0.5", "(monitor_h / 2) + 17" }

-- Set windiw suze to the half monitor resolution
size = { "monitor_w * 0.5", "monitor_h * 0.5" }

-- Set max_size as half of the monitor width and height
max_size = "monitor_w*0.5 monitor_h*0.5"
```

{{% /details %}}

### Dynamic effects

Dynamic effects are re-evaluated every time a property changes.

<!-- SORT: opaque and force_rgbx are together because they do very similar -->
| Effect | Description | Argument | Limits |
| --- | --- | --- | --- |
| allows_input | Forces an Xwayland window to receive input even if it requests not to | bool | |
| animation | Forces an animation onto a window with an optional style (e.g., `"popin"`, `"popin 80%"`) | string | |
| border_color | Sets the window's border to a color or gradient. See [border_color](#border_color-window-rule) below | gradient | |
| border_size | Sets the border size | int | |
| confine_pointer | Locks the mouse cursor to the window. Mostly useful for keeping your mouse cursor locked to one monitor during gaming | bool | |
| dim_around | Dims everything around the window. Meant for floating windows | bool | |
| decorate | Whether to draw window decorations. (default: `true`) | bool | |
| focus_on_activate | Whether Hyprland should focus an app that requests to be focused | bool | |
| idle_inhibit | Sets an idle inhibit rule. Modes: `"none"`, `"always"`, `"focus"`, `"fullscreen"` | string | |
| immediate | Forces the window to allow tearing | bool | |
| keep_aspect_ratio | Forces aspect ratio when resizing with the mouse | bool | |
| max_size | Sets the maximum size for floating windows. See [position and size](#position-and-size) above | table or string | |
| min_size | Sets the minimum size for floating windows. See [position and size](#position-and-size) above | table or string | |
| nearest_neighbor | Forces nearest-neighbor filtering | bool | |
| no_anim | Disables animations for the window | bool | |
| no_auto_hdr | Disables AutoHDR for the window. This is useful to stop programs like `foot` triggering AutoHDR when they are fullscreened | bool | |
| no_blur | Disables blur for the window | bool | |
| no_dim | Disables window dimming for the window | bool | |
| no_focus | Disables focus to the window | bool | |
| no_follow_mouse | Prevents the window from being focused when the mouse moves over it when `input.follow_mouse=1` is set | bool | |
| no_glow | Disables glow for the window | bool | |
| no_max_size | Removes max size limitations | bool | |
| no_screen_share | Hides the window and its popups from screen sharing by drawing black rectangles in their place | bool | |
| no_shadow | Disables shadows for the window | bool | |
| no_shortcuts_inhibit | Disallows the app from inhibiting your shortcuts | bool | |
| no_vrr | Disables VRR for the window. Only works when `misc.vrr` is set to `2` or `3` | bool | |
| no_wobble | Disables wobble for the window | bool | |
| no_xdg_drags | If true, will disable XDG-driven drags for the window (e.g., dragging a CSD top bar) | bool | |
| opacity | Additional opacity multiplier (e.g., `"0.8"` overall, `"0.9 0.7"` active/inactive, `"1.0 0.8 0.9"` active/inactive/fullscreen). Append `" override"` after each value to set absolute instead of multiplied | string | |
| opaque | Set `decoration.active/inactive/fullscreen_opacity` to `1` for this window | bool | |
| force_rgbx | Disables the window's own transparency *(not Hyprland's given transparency, in contrast to the opaque rule)* | bool | |
| persistent_size | For floating windows, internally store their size. When a new floating window opens with the same class and title, restore the saved size | bool | |
| render_unfocused | Forces the window to think it's being rendered when it's not visible | bool | |
| rounding | Forces X pixels of rounding, ignoring the default | int | |
| rounding_power | Overrides the rounding power for the window | float | |
| scroll_mouse | Forces the window to override `input.scroll_factor` | float | |
| scroll_touchpad | Forces the window to override `input.touchpad.scroll_factor` | float | |
| stay_focused | Forces focus on the window as long as it's visible | bool | |
| sync_fullscreen | Whether the fullscreen mode should always be the same as the one sent to the window | bool | |
| tag | Applies a tag. Use prefix `+`/`-` to set/unset, or no prefix to toggle (e.g., `"+myTag"`) | string | |
| tonemap | Tonemapping behavior: `on` (Default), `off` disables tonemapping, `clamp` clamps source luminance to target, `limited` uses a dynamic curve to tonemap only the top end out of bounds content | string | |
| xray | Sets blur xray mode for the window | bool | |

All dynamic effects can be set with [`set_prop()`](../../dispatchers#set_prop).

