-- Default FieldAssist init script.
-- Dump with: FieldAssist --dump-init
-- Copy to the app config directory to customize.

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
    return chosen
  end
  local n = c.channels
  if n == 1 then
    return "mono"
  elseif n == 2 then
    return "stereo"
  elseif n == 4 then
    return "1OA"
  elseif n == 9 then
    return "2OA"
  end
end)
