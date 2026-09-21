-- Default FieldAssist init script.
-- Dump with: FieldAssist --dump-init
-- Copy to the app config directory to customize.
--
-- Optional: pin an output device across launches (substring match).
-- app.output_device = "Focusrite"

-- Optional: choose appearance (default is dark).
-- app.theme.mode = "light"
-- app.theme.name = "Catppuccin Mocha"
-- See app.themes for the full list.

local started = os.clock()

app:define_layout({
  name = "mono",
  description = "Single channel",
  channels = { [0] = "Mono" },
  monitor = { chain = "mono" },
})

app:define_layout({
  name = "stereo",
  description = "Left / Right",
  channels = { [0] = "L", [1] = "R" },
  monitor = { chain = "stereo" },
})

app:define_layout({
  name = "MS",
  description = "Mid / Side",
  channels = { [0] = "M", [1] = "S" },
  monitor = { chain = "ms" },
})

app:define_layout({
  name = "B-Format (AmbiX)",
  description = "First-order Ambisonics (Ambix ACN/SN3D)",
  channels = { [0] = "W", [1] = "Y", [2] = "Z", [3] = "X" },
  monitor = { chain = "foa" },
})

app:define_layout({
  name = "B-Format (FuMa)",
  description = "First-order Ambisonics (Furse-Malham)",
  channels = { [0] = "W", [1] = "X", [2] = "Y", [3] = "Z" },
  monitor = { chain = "foa_fuma" },
})

app:define_layout({
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

app:on("detect_layout", function(c, chosen)
  if chosen == "1OA" then
    chosen = "B-Format (AmbiX)"
  end
  if chosen then
    app:info("layout", chosen)
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
    app:info("layout", name)
  else
    app:info("layout", "none")
  end
  return name
end)

app:on("loaded", function(c, elapsed)
  app:info("load", string.format("%s in %.2f ms", c.name, elapsed * 1000))
end)

app:on("saved", function(c, elapsed)
  app:info("save", string.format("%s in %.2f ms", c.name, elapsed * 1000))
end)

app:info("init", string.format("evaluated in %.2f ms", (os.clock() - started) * 1000))
