#!/usr/bin/env field-batch
--
-- SPDX-FileCopyrightText: 2026 Greg Wuller
-- SPDX-License-Identifier: MIT
--
-- Index audio files under a directory into a SQLite database using a
-- locally built lsqlite3.so (see README.md / Makefile in this directory).
--
-- Requires `field-batch` on PATH and `./lsqlite3.so` next to the cwd
-- (run from this directory after `make`):
--
--   cd docs/examples/field-batch/lsqlite-module
--   make
--   field-batch index-audio-files.lua /path/to/media ./audio-files.sqlite

field.scripting.enable_native_modules()

local ok, sqlite3 = pcall(require, "lsqlite3")
if not ok then
  error(
    "require('lsqlite3') failed: "
      .. tostring(sqlite3)
      .. "\nBuild the module first (`make` in this directory) and run with cwd "
      .. "set here so ./lsqlite3.so is on package.cpath. Rebuild field-batch "
      .. "so the host exports Lua symbols (-rdynamic).",
    0
  )
end

local MEDIA_EXTS = {
  "wav",
  "wave",
  "aif",
  "aiff",
  "flac",
  "ogg",
  "oga",
  "mp3",
  "mp2",
  "m4a",
  "aac",
  "caf",
  "w64",
}

local root = app.args[1] or "."
local db_path = app.args[2] or "audio-files.sqlite"

if not field.fs.exists(root) then
  error("media root does not exist: " .. tostring(root), 0)
end

local files = field.fs.find_files(root, MEDIA_EXTS)
local root_url = field.url.from_path(root)

local db = assert(sqlite3.open(db_path))
local rc = db:exec([[
CREATE TABLE IF NOT EXISTS audio_files (
  path TEXT PRIMARY KEY NOT NULL
);
]])
if rc ~= sqlite3.OK then
  local msg = db:errmsg()
  db:close()
  error("CREATE TABLE failed: " .. tostring(msg), 0)
end

local insert = assert(db:prepare("INSERT OR REPLACE INTO audio_files (path) VALUES (?)"))
for _, rel in ipairs(files) do
  local abs = root_url:join(rel).native
  insert:bind_values(abs)
  if insert:step() ~= sqlite3.DONE then
    local msg = db:errmsg()
    insert:finalize()
    db:close()
    error("INSERT failed: " .. tostring(msg), 0)
  end
  insert:reset()
end
insert:finalize()
db:close()

print(string.format("wrote %d path(s) to %s", #files, db_path))
