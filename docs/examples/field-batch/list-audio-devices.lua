#!/usr/bin/env field-batch
-- SPDX-FileCopyrightText: 2026 Greg Wuller
-- SPDX-License-Identifier: MIT
--
-- List playback output devices and print each property field.audio_devices
-- exposes. Requires `field-batch` on PATH (e.g. after
-- `./script/bundle-macos --install`).
--
--   ./docs/examples/field-batch/list-audio-devices.lua
--   field-batch docs/examples/field-batch/list-audio-devices.lua

local devices = field.audio_devices.list()
if #devices == 0 then
  print("(no output audio devices)")
  return
end

local function sorted_keys(row)
  local keys = {}
  for key in pairs(row) do
    keys[#keys + 1] = key
  end
  table.sort(keys)
  return keys
end

local function format_value(value)
  local ty = type(value)
  if ty == "nil" then
    return "nil"
  elseif ty == "string" then
    return string.format("%q", value)
  elseif ty == "boolean" or ty == "number" then
    return tostring(value)
  end
  return string.format("<%s>", ty)
end

print(string.format("%d output device(s)  (app.name=%s)", #devices, app.name))
print()

for i, device in ipairs(devices) do
  local title = device.name or ("device #" .. tostring(i))
  if device.is_default then
    title = title .. "  [default]"
  end
  print(string.format("%d. %s", i, title))
  for _, key in ipairs(sorted_keys(device)) do
    if key ~= "name" then
      print(string.format("   %-14s %s", key, format_value(device[key])))
    end
  end
  print()
end
