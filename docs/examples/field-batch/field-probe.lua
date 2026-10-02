#!/usr/bin/env field-batch
-- SPDX-FileCopyrightText: 2026 Greg Wuller
-- SPDX-License-Identifier: MIT
--
-- Probe each media path on the command line and print technical media
-- fields plus every source / source.* variable (Variables pane probe
-- metadata). Requires `field-batch` on PATH (e.g. after
-- `./script/bundle-macos --install`).
--
-- Usage:
--   ./docs/examples/field-batch/field-probe.lua take.wav other.flac
--   field-batch docs/examples/field-batch/field-probe.lua take.wav

if #app.args == 0 then
  error(
    "usage: field-batch field-probe.lua <media> [media…]",
    0
  )
end

local MEDIA_KEYS = {
  "id",
  "basename",
  "sample_rate",
  "channels",
  "frames",
  "bit_depth",
  "size_bytes",
  "modified",
  "container_format",
  "codec",
  "duration",
}

local function sorted_keys(names)
  local keys = {}
  for _, name in ipairs(names) do
    keys[#keys + 1] = name
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
  elseif ty == "userdata" then
    local ok, path = pcall(function()
      return value:as_path()
    end)
    if ok and type(path) == "string" then
      return string.format("%q", path)
    end
    return "<userdata>"
  end
  return string.format("<%s>", ty)
end

local function is_source_scope(scope)
  return scope == "source" or scope:match("^source%.") ~= nil
end

local function probe_one(path)
  if not field.fs.exists(path) then
    error("source does not exist: " .. tostring(path), 0)
  end

  local c = field.composition.open(path)
  local media = c.media[1]
  if not media then
    error("no media after open: " .. tostring(path), 0)
  end

  print(string.format("=== %s ===", path))
  print("media:")
  for _, key in ipairs(MEDIA_KEYS) do
    print(string.format("  %-18s %s", key, format_value(media[key])))
  end
  print(string.format("  %-18s %s", "url", format_value(media.url)))

  local source_bindings = {}
  for _, b in ipairs(c:variable_resolver():bindings()) do
    if is_source_scope(b:scope()) then
      source_bindings[#source_bindings + 1] = b
    end
  end

  if #source_bindings == 0 then
    print("source variables: (none)")
    return
  end

  print("source variables:")
  for _, b in ipairs(source_bindings) do
    local scope = b:scope()
    local names = sorted_keys(b:names())
    if #names == 0 then
      print(string.format("  [%s]  (empty)", scope))
    else
      print(string.format("  [%s]", scope))
      for _, name in ipairs(names) do
        print(string.format("    %-28s %s", name, format_value(b.values[name])))
      end
    end
  end
end

for i, path in ipairs(app.args) do
  if i > 1 then
    print()
  end
  probe_one(path)
end
