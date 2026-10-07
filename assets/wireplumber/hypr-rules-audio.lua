-- WirePlumber policy: per-program audio routes managed by Hyprland Windows Rules.
--
-- A route sends a program's playback and/or capture streams to a chosen device,
-- e.g. Discord + Mumble -> "Maxwell Chat" / "Maxwell Mic". Devices are matched by
-- node.description or a node.name glob, because some devices (Audeze Maxwell)
-- change node.name between connection modes while keeping their description.
--
-- Routes live in the "hypr-rules-audio" metadata (subject 0, key "routes"), so the
-- app can change them live with pw-metadata. Scripts can't read files (Lua
-- sandbox), so the initial value comes from the hypr-rules.audio-routes section
-- of this script's .conf, which the app keeps in sync.
--
-- Routes are authoritative: they override a target the app itself asked for and
-- one restored from a manual move. Streams without a route, and streams tagged by
-- LutrisToSunshine, are left to WirePlumber. A missing device falls through to the
-- normal default; when it reappears the linkable rescan moves the stream back.

local lutils = require ("linking-utils")
local log = Log.open_topic ("s-hypr-rules-audio")

local METADATA = "hypr-rules-audio"
local routes = {}

local function parse_routes (value)
  if not value then return {} end
  local ok, parsed = pcall (function () return Json.Raw (value):parse () end)
  if not ok or type (parsed) ~= "table" then
    log:warning ("ignoring malformed routes: " .. tostring (value))
    return {}
  end
  return parsed
end

-- "a*b" -> "^a.*b$" with every other character literal
local function glob_to_pattern (glob)
  local escaped = glob:gsub ("[%^%$%(%)%%%.%[%]%+%-%?]", "%%%0")
  return "^" .. escaped:gsub ("%*", ".*") .. "$"
end

local function device_matches (props, device)
  if device.description and props ["node.description"] == device.description then
    return true
  end
  local name = props ["node.name"]
  if device.name and name == device.name then return true end
  if device.pattern and name and name:match (glob_to_pattern (device.pattern)) then
    return true
  end
  return false
end

-- The program behind a stream. Streams through the PulseAudio layer carry the
-- binary; ones through PipeWire's ALSA plugin only say "alsa_playback.<program>"
-- (Mumble's ALSA output does this).
local function program_of (props)
  local binary = props ["application.process.binary"]
  if binary then return binary end
  local name = props ["node.name"] or ""
  return name:match ("^alsa_playback%.(.+)$") or name:match ("^alsa_capture%.(.+)$")
      or (props ["application.name"] or ""):match ("^PipeWire ALSA %[(.+)%]$")
end

local function one_of (actual, wanted)
  local values = type (wanted) == "table" and wanted or { wanted }
  for _, v in ipairs (values) do
    if actual == v then return true end
  end
  return false
end

-- A route applies through its program list and/or extra property matches; it
-- needs at least one of them.
local function stream_matches (props, route)
  local any = false
  if route.programs then
    local program = program_of (props)
    if not program or not one_of (program, route.programs) then return false end
    any = true
  end
  for key, wanted in pairs (route.match or {}) do
    local actual = props [key]
    if actual == nil or not one_of (actual, wanted) then return false end
    any = true
  end
  return any
end

-- The device this stream should use, or nil when no route applies.
local function wanted_device (props)
  local class = props ["media.class"] or ""
  local side
  if class == "Stream/Output/Audio" then side = "output"
  elseif class == "Stream/Input/Audio" then side = "input"
  else return nil end
  for _, route in ipairs (routes) do
    if route.enabled ~= false and route [side] and stream_matches (props, route) then
      return route [side], side
    end
  end
  return nil
end

SimpleEventHook {
  name = "hypr-rules-audio/route",
  after = "linking/find-defined-target",
  before = { "linking/find-audio-group-target", "linking/find-filter-target",
             "linking/find-media-role-target", "linking/find-default-target",
             "linking/find-best-target", "linking/prepare-link" },
  interests = {
    EventInterest {
      Constraint { "event.type", "=", "select-target" },
    },
  },
  execute = function (event)
    local source, om, si, si_props, si_flags = lutils:unwrap_select_target_event (event)
    if si_props ["lutristosunshine.stream"] then return end
    local device, side = wanted_device (si_props)
    if not device then return end

    local want_class = side == "output" and "Audio/Sink" or "Audio/Source"
    for lnkbl in om:iterate { type = "SiLinkable" } do
      local p = lnkbl.properties
      if p ["media.class"] == want_class and device_matches (p, device)
          and lutils.canLink (si_props, lnkbl) then
        local compatible, can_passthrough = lutils.checkPassthroughCompatibility (si, lnkbl)
        if compatible then
          log:info (si, string.format ("route %s -> %s",
              tostring (program_of (si_props)),
              tostring (p ["node.description"])))
          si_flags.has_defined_target = true
          si_flags.has_node_defined_target = false
          si_flags.can_passthrough = can_passthrough
          event:set_data ("target", lnkbl)
          return
        end
      end
    end
    log:debug (si, "routed device not present; leaving the stream to the default")
  end,
}:register ()

-- Route changes from the app: re-read and re-place every stream.
SimpleEventHook {
  name = "hypr-rules-audio/routes-changed",
  interests = {
    EventInterest {
      Constraint { "event.type", "=", "metadata-changed" },
      Constraint { "metadata.name", "=", METADATA },
    },
  },
  execute = function (event)
    local props = event:get_properties ()
    if props ["event.subject.key"] ~= "routes" then return end
    routes = parse_routes (props ["event.subject.value"])
    log:info (string.format ("%d audio route(s) loaded", #routes))
    event:get_source ():call ("schedule-rescan", "linking")
  end,
}:register ()

local initial = Conf.get_section_as_json ("hypr-rules.audio-routes", Json.Array {})
routes = parse_routes (initial:to_string ())

metadata = ImplMetadata (METADATA)
metadata:activate (Features.ALL, function (m, e)
  if e then
    log:warning ("failed to activate the " .. METADATA .. " metadata: " .. tostring (e))
    return
  end
  m:set (0, "routes", "Spa:String:JSON", initial:to_string ())
end)

log:info (string.format ("hypr-rules audio routing registered (%d route(s))", #routes))
