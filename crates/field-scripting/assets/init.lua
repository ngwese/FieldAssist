-- Default FieldAssist / field-play init script.
-- Dump with: FieldAssist --dump-init
-- Copy to the app config directory to customize.
--
-- Note: field.variables loads resolver_default.lua before this file, so the
-- `"default"` last-wins resolver is already declared. Override with
-- field.variables.declare_resolver / set_resolver in this file if needed.
--
-- Hosts apply settings.json (scripting / device / experimental, and FieldAssist
-- application + waveform) from Rust before this file runs. Prefer Settings UI
-- or settings.json over app:load_settings() at startup; that method remains for
-- manual reload from scripts.

-- Optional: pin an output device across launches (substring match).
-- app.output_device = "Focusrite"

-- Optional: choose appearance (default is dark).
-- app.theme.mode = "light"
-- app.theme.name = "Catppuccin Mocha"
-- See app.themes for the full list.

local started = os.clock()

--
-- Channel Layouts
--

field.layouts.define({
  name = "mono",
  code = "mn",
  description = "Single channel",
  channels = { [0] = "C" },
  monitor = { chain = "mono" },
})

field.layouts.define({
  name = "stereo",
  code = "st",
  description = "Left / Right",
  channels = { [0] = "L", [1] = "R" },
  monitor = { chain = "stereo" },
})

field.layouts.define({
  name = "MS",
  code = "ms",
  description = "Mid / Side",
  channels = { [0] = "M", [1] = "S" },
  monitor = { chain = "ms" },
})

field.layouts.define({
  name = "B-Format (AmbiX)",
  code = "ambix",
  description = "First-order Ambisonics (Ambix ACN/SN3D)",
  channels = { [0] = "W", [1] = "Y", [2] = "Z", [3] = "X" },
  monitor = { chain = "foa" },
})

field.layouts.define({
  name = "B-Format (FuMa)",
  code = "fuma",
  description = "First-order Ambisonics (Furse-Malham)",
  channels = { [0] = "W", [1] = "X", [2] = "Y", [3] = "Z" },
  monitor = { chain = "foa_fuma" },
})

field.layouts.define({
  name = "2OA",
  description = "Second-order Ambisonics (Ambix)",
  channels = {
    [0] = "W",
    [1] = "Y",
    [2] = "Z",
    [3] = "X",
    [4] = "V",
    [5] = "T",
    [6] = "R",
    [7] = "S",
    [8] = "U",
  },
})

--
-- Export Profiles
--

field.exports.define({
  name = "WAV (Source Equivalent)",
  description = "WAV matching source rate, format, and channels",
  encoder = "wav",
})

field.exports.define({
  name = "WAV (48kHz)",
  description = "WAV at 48 kHz, source equivalent format",
  encoder = "wav",
  sample_rate = 48000,
})

field.exports.define({
  name = "FLAC (Source Equivalent)",
  description = "FLAC matching source rate, format, and channels",
  encoder = "flac",
})

field.exports.define({
  name = "FLAC (48kHz)",
  description = "FLAC at 48 kHz, source equivalent format",
  encoder = "flac",
  sample_rate = 48000,
})

--
-- Event Handlers
--

-- Event name from the parent folder (e.g. …/2026-10-04/take.wav → "2026-10-04").
-- Ingest joins staging/backup under ${source.event.name}. Override in a
-- config-directory init.lua or a later enrich_media hook.
field.on("enrich_media", function(m)
  local parent = m.url and m.url.parent
  local name = parent and parent.basename
  if name and name ~= "" then
    m:bindings("source.event").values.name = name
  end
end)

field.on("detect_layout", function(c, chosen)
  if chosen == "1OA" then
    chosen = "B-Format (AmbiX)"
  end
  if chosen then
    field.log.info("layout", chosen)
    return chosen
  end
  local n = c.channels
  local base = string.lower(c.basename or "")
  local name
  if n >= 4 and string.find(base, "fuma", 1, true) then
    name = "B-Format (FuMa)"
  elseif n >= 4 and string.find(base, "ambix", 1, true) then
    name = "B-Format (AmbiX)"
  elseif n == 1 then
    name = "mono"
  elseif n == 2 then
    name = "stereo"
  elseif n == 4 then
    name = "B-Format (AmbiX)"
  elseif n == 9 then
    name = "2OA"
  end
  if name then
    field.log.info("layout", name)
  else
    field.log.info("layout", "none")
  end
  return name
end)

field.on("loaded", function(c, elapsed)
  field.log.info("load", string.format("%s in %.2f ms", c.name, elapsed * 1000))
end)

field.on("saved", function(c, elapsed)
  field.log.info("save", string.format("%s in %.2f ms", c.name, elapsed * 1000))
end)

field.log.info("init", string.format("evaluated in %.2f ms", (os.clock() - started) * 1000))
