-- Default FieldAssist init script.
-- Dump with: FieldAssist --dump-init
-- Copy to the app config directory to customize.

local started = os.clock()

app:define_layout({
  name = "mono",
  description = "Single channel",
  channels = { [0] = "Mono" },
})

app:define_layout({
  name = "stereo",
  description = "Left / Right",
  channels = { [0] = "L", [1] = "R" },
})

app:define_layout({
  name = "MS",
  description = "Mid / Side",
  channels = { [0] = "M", [1] = "S" },
})

app:define_layout({
  name = "1OA",
  description = "First-order Ambisonics (Ambix)",
  channels = { [0] = "W", [1] = "Y", [2] = "Z", [3] = "X" },
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
  if chosen then
    app:info("layout", chosen)
    return chosen
  end
  local n = c.channels
  local name
  if n == 1 then
    name = "mono"
  elseif n == 2 then
    name = "stereo"
  elseif n == 4 then
    name = "1OA"
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
  app:info("load", string.format("%s in %.0f ms", c.name, elapsed * 1000))
end)

app:on("saved", function(c, elapsed)
  app:info("save", string.format("%s in %.0f ms", c.name, elapsed * 1000))
end)

app:info("init", string.format("evaluated in %.0f ms", (os.clock() - started) * 1000))
