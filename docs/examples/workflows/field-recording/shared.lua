-- Spec example helpers for the field-recording pipeline.
-- See docs/spec/SPEC-field-recording.md.
--
-- NOT embedded. NOT expected to run until intended host APIs ship.
-- Copy into the FieldAssist config directory next to workflow_*.lua.

local M = {}

-- Readable audio. Review also expands `.facomp` via `review_exts`.
local MEDIA_EXTS = {
  "wav", "wave", "aif", "aiff", "flac", "ogg", "oga",
  "mp3", "mp2", "m4a", "aac", "caf", "w64",
}

local REVIEW_EXTS = {
  "wav", "wave", "aif", "aiff", "flac", "ogg", "oga",
  "mp3", "mp2", "m4a", "aac", "caf", "w64", "facomp",
}

function M.media_exts()
  return MEDIA_EXTS
end

function M.review_exts()
  return REVIEW_EXTS
end

function M.get_prop(session, key, default)
  local props = session.properties or {}
  local value = props[key]
  if value == nil or value == "" then
    return default or ""
  end
  return value
end

function M.set_prop(session, key, value)
  local props = session.properties or {}
  props[key] = value or ""
  session.properties = props
end

-- Intended: app.url(native_or_url) with :join, :parent, :basename, :stem,
-- :extension, :with_extension, :relative_to, .native, .normalized.
-- Until that ships, path joining is a best-effort string helper for sketches.
function M.join_path(dir, rel)
  if not rel or rel == "" then
    return dir
  end
  local last = dir:sub(-1)
  local sep = "/"
  if last == "/" or last == "\\" then
    sep = ""
  elseif dir:find("\\") then
    sep = "\\"
  end
  if sep == "\\" then
    rel = rel:gsub("/", "\\")
  end
  return dir .. sep .. rel
end

function M.is_session_path(path)
  return string.lower(path or ""):match("%.fasession$") ~= nil
end

function M.expand_media(path, exts)
  exts = exts or MEDIA_EXTS
  local ok, found = pcall(function()
    return app:find_files(path, exts)
  end)
  if not ok then
    return { path }
  end
  local paths = {}
  for _, rel in ipairs(found) do
    paths[#paths + 1] = M.join_path(path, rel)
  end
  return paths
end

function M.docs_in_group(session, group)
  local docs = {}
  for _, doc in ipairs(session.compositions or {}) do
    if doc.group == group then
      docs[#docs + 1] = doc
    end
  end
  return docs
end

function M.ensure_command_flag(id, want)
  local current = (id == "transport.loop") and app.looping or app.preview
  if current ~= want then
    app:command(id)
  end
end

-- Intended ledger layout (app.sqlite.open under staging_root or next to session):
--
--   CREATE TABLE IF NOT EXISTS ingest (
--     source_url TEXT PRIMARY KEY,
--     backup_url TEXT,
--     staging_url TEXT,
--     source_checksum TEXT,
--     staging_checksum TEXT,
--     ingest_status TEXT,
--     catalog_status TEXT,
--     error TEXT
--   );
--
-- Example once app.sqlite exists:
--
--   local db = app.sqlite.open(ledger_url)
--   db:exec([[CREATE TABLE IF NOT EXISTS ingest (...)]])
--   db:exec(
--     "INSERT OR REPLACE INTO ingest (source_url, ingest_status) VALUES (?, ?)",
--     { source_url, "pending" }
--   )
--   local rows = db:query("SELECT * FROM ingest WHERE ingest_status != ?", { "verified" })
--   db:close()

function M.ledger_path(staging_root)
  -- Intended: app.url(staging_root):join("ingest.sqlite").native
  return M.join_path(staging_root, "ingest.sqlite")
end

-- Intended filesystem sketch (app.fs):
--
--   local src = app.url(source_path)
--   local dest = app.url(backup_root):join(src:relative_to(source_root))
--   app.fs.mkdir(dest.parent, { recursive = true })
--   app.fs.copy(src, dest)
--   local sum = app.fs.checksum(dest)
--
-- Intended convert sketch (metadata + encode):
--
--   local tags = app.media_tags.read(src)           -- container fields
--   local canonical = app.media_tags.canonical(tags) -- created_on, created_by, …
--   app.media_convert(src, staging_url, {
--     format = "flac",
--     tags = canonical,
--   })
--   -- Future: app.c2pa.sign(staging_url, { ... })  -- derivative only, not backup

return M
