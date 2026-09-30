#!/usr/bin/env field-batch
-- SPDX-FileCopyrightText: 2026 Greg Wuller
-- SPDX-License-Identifier: MIT
--
-- Transcode a media file into `user.ingest.staging_dir` using
-- `field.media.open` + `field.media.transcode`. Creates the staging
-- directory with `field.fs.mkdir` when it is missing.
--
-- Set the staging path in the FieldAssist config `variables.json`, e.g.:
--
--   {
--     "kind": "variables",
--     "format_version": 1,
--     "variables": [
--       {
--         "name": "ingest.staging_dir",
--         "value": "${env.HOME}/Music/FieldAssist/staging"
--       }
--     ]
--   }
--
-- (Dots in `name` become user sub-scopes → `user.ingest.staging_dir`.
--  `:resolve(..., true)` soft-expands `${…}` in the value.)
--
-- Usage (requires `field-batch` on PATH):
--
--   field-batch docs/examples/field-batch/transcode-to-staging.lua \
--     /path/to/take.wav
--   field-batch docs/examples/field-batch/transcode-to-staging.lua \
--     /path/to/take.wav "FLAC (Source Equivalent)"
--
-- Optional second argument: export profile name (must be registered via
-- `field.exports.define` / init.lua). Defaults to "FLAC (Source Equivalent)"
-- from the embedded init profiles.

local src_path = app.args[1]
if not src_path or src_path == "" then
  error(
    "usage: field-batch transcode-to-staging.lua <media> [export-profile-name]",
    0
  )
end

if not field.fs.exists(src_path) then
  error("source does not exist: " .. tostring(src_path), 0)
end

local staging, scope = field.variables
  .user()
  :variable_resolver()
  :resolve("user.ingest.staging_dir", true)
if not staging or staging == "" then
  error(
    "user.ingest.staging_dir is unset; add ingest.staging_dir to variables.json",
    0
  )
end
print(string.format("staging_dir=%s  (from %s)", staging, scope or "?"))

if not field.fs.exists(staging) then
  print("creating staging directory: " .. staging)
  field.fs.mkdir(staging)
end

local profile_name = app.args[2]
if not profile_name or profile_name == "" then
  profile_name = "FLAC (Source Equivalent)"
end
local profile = field.exports.shared_registry():find(profile_name)
if not profile then
  error("unknown export profile: " .. tostring(profile_name), 0)
end

local src = field.media.open(src_path)
local dest = field.url
  .from_path(staging)
  :join(src.url.stem .. "." .. profile.extension)
  :as_path()

print(string.format(
  "transcode %s → %s  (%d frames @ %d Hz, %d ch)",
  src.url.basename,
  dest,
  src.frames,
  src.sample_rate,
  src.channels
))

local last_pct = -1
field.media.transcode(src, dest, profile, function(done, total)
  if total <= 0 then
    return
  end
  local pct = math.floor((done * 100) / total)
  if pct ~= last_pct and (pct % 10 == 0 or done == total) then
    last_pct = pct
    print(string.format("  %3d%%  (%d / %d frames)", pct, done, total))
  end
end)

local out = field.media.open(dest)
print(string.format(
  "done: %s  (%d frames @ %d Hz, %s)",
  dest,
  out.frames,
  out.sample_rate,
  out.container_format
))
