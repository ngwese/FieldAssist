-- Catalog workflow for the field-recording pipeline.
-- See docs/spec/SPEC-field-recording.md and docs/SCRIPTING.md.
--
-- Modal sheet: Configure → Run → Summary → Cleanup.
-- Run also copies kept `.facomp` files and the focused `.fasession` into
-- each library event directory beside the rendered samples.
-- Configure mirrors Ingest: library path with muted resolved preview,
-- composition count/size, and export profile. Run uses deferred polling
-- (like Ingest transcode) so the UI stays responsive with composition-
-- level and per-render progress bars plus an Issues log. Summary shows
-- rendered file counts/sizes/destination. Cleanup lists staging files
-- with Cancel + alert-colored Cleanup (empty parent dirs under staging
-- are removed too; summary notes file + directory counts).
--
-- Copy into the FieldAssist config directory next to init.lua.
-- Requires shared.lua in the same directory (via field.include).
-- Run after Review has marked documents keep / drop.

local shared = field.include("shared.lua")

local MIXDOWN_CHAINS = {
  ambix = "foa",
  fuma  = "foa_fuma",
  ms    = "ms",
}

local DEFAULT_PROFILE = "FLAC (Source Equivalent)"

local Catalog = field.workflow.create({
  name = "catalog",
  display_name = "Catalog",
  description = "Export kept takes into the library and clean staging",
  scopes = { "menu" },
})

function Catalog:init()
  -- Prefer session catalog.library_root; resolve falls back to user.
  self.library_root = "${catalog.library_root}"
  self.staging_root = ""
  self.profile = DEFAULT_PROFILE
  self.busy = false
  self.error_count = 0
  self.warning_count = 0
  self.run_results = {}
  self.written_paths = {}
  self.written_bytes = 0
end

-- ── Formatting / progress helpers (Ingest-shaped) ─────────────────────────────

function Catalog:format_bytes(n)
  n = tonumber(n) or 0
  if n < 1024 then
    return string.format("%d B", n)
  elseif n < 1024 * 1024 then
    return string.format("%.1f KB", n / 1024)
  elseif n < 1024 * 1024 * 1024 then
    return string.format("%.1f MB", n / (1024 * 1024))
  end
  return string.format("%.2f GB", n / (1024 * 1024 * 1024))
end

function Catalog:set_comps_progress(done, total, caption)
  if not self.comps_progress then
    return
  end
  total = math.max(tonumber(total) or 0, 0)
  done = math.max(tonumber(done) or 0, 0)
  if total <= 0 then
    self.comps_progress.loading = true
    self.comps_progress.text = caption or "Idle"
    return
  end
  self.comps_progress.loading = false
  self.comps_progress.value = math.min(100, (done / total) * 100)
  self.comps_progress.text = caption
    or string.format("%d / %d composition(s)", done, total)
end

function Catalog:set_render_progress(fraction, caption)
  if not self.render_progress then
    return
  end
  if fraction == nil then
    self.render_progress.loading = true
    self.render_progress.text = caption or "…"
    return
  end
  self.render_progress.loading = false
  self.render_progress.value = math.max(0, math.min(100, (tonumber(fraction) or 0) * 100))
  self.render_progress.text = caption
    or string.format("%d%%", math.floor(self.render_progress.value + 0.5))
end

function Catalog:set_current_comp(doc)
  if not self.current_comp then
    return
  end
  if not doc then
    self.current_comp.text = "—"
    return
  end
  local name = doc.name or "?"
  local size = self:format_bytes(self:doc_size_bytes(doc))
  local dur = tonumber(doc.duration) or 0
  local length
  if dur >= 3600 then
    length = string.format(
      "%d:%02d:%02d",
      math.floor(dur / 3600),
      math.floor((dur % 3600) / 60),
      math.floor(dur % 60)
    )
  else
    length = string.format("%d:%02d", math.floor(dur / 60), math.floor(dur % 60))
  end
  self.current_comp.text = string.format("%s  ·  %s  ·  %s", name, size, length)
end

function Catalog:log_issue(level, message)
  field.log[level]("catalog", message)
  if level == "error" then
    self.error_count = (self.error_count or 0) + 1
  elseif level == "warn" then
    self.warning_count = (self.warning_count or 0) + 1
  end
  if self.log then
    self.log:append(string.format("%s: %s", level, message))
  end
end

-- ── Path / variable resolution ────────────────────────────────────────────────

--- Expand path templates like Ingest: whole `${name}`, bare `user.*` /
--- `catalog.*`, inline `${…}`, or a concrete path.
function Catalog:resolve_path_or_var(raw, fallback_name)
  if not raw or raw == "" then
    raw = fallback_name
  end
  if not raw or raw == "" then
    return nil
  end
  local resolver = field.variables.user():variable_resolver()
  local name = raw:match("^%$%{(.+)%}$")
  if name then
    local value = resolver:resolve(name, true)
    -- catalog.library_root empty → user.catalog.library_root
    if (not value or value == "") and name == "catalog.library_root" then
      value = resolver:resolve("user.catalog.library_root", true)
    end
    return value
  end
  if raw:match("^user%.") or raw:match("^catalog%.") then
    local value = resolver:resolve(raw, true)
    if (not value or value == "") and raw == "catalog.library_root" then
      value = resolver:resolve("user.catalog.library_root", true)
    end
    return value
  end
  if raw:find("${", 1, true) then
    return resolver:expand(raw)
  end
  return raw
end

function Catalog:resolved_library()
  return self:resolve_path_or_var(self.library_root, "catalog.library_root")
end

function Catalog:preview_path(raw, fallback_name)
  local resolved = self:resolve_path_or_var(raw, fallback_name)
  if not resolved or resolved == "" then
    return "—"
  end
  return resolved
end

function Catalog:resolve_staging_root()
  local raw = self.staging_root
  if not raw or raw == "" then
    local session = field.session.focused()
    raw = shared.get_prop(session, "staging_root")
  end
  -- Same fallback as Ingest: session prop may be ${user.ingest.staging_root}.
  return self:resolve_path_or_var(raw, "user.ingest.staging_root")
end

function Catalog:persist(session)
  shared.set_prop(session, "catalog_library_root", self.library_root or "")
  if self.staging_root and self.staging_root ~= "" then
    shared.set_prop(session, "staging_root", self.staging_root)
  end
end

function Catalog:restore(session)
  local saved = shared.get_prop(session, "catalog_library_root")
  if saved and saved ~= "" then
    self.library_root = saved
  elseif not self.library_root or self.library_root == "" then
    self.library_root = "${catalog.library_root}"
  end
  self.staging_root = self.staging_root ~= "" and self.staging_root
    or shared.get_prop(session, "staging_root")
    or ""

  local resolver = field.variables.user():variable_resolver()
  local ok, profile = pcall(function()
    return resolver:resolve("catalog.export_profile", true)
  end)
  if ok and profile and profile ~= "" then
    self.profile = profile
  end
end

-- ── Size / selection totals ───────────────────────────────────────────────────

function Catalog:doc_size_bytes(doc)
  local total = 0
  local ok, media_list = pcall(function()
    return doc.media
  end)
  if ok and media_list then
    for _, m in ipairs(media_list) do
      local sok, sz = pcall(function()
        return m.size_bytes
      end)
      if sok and tonumber(sz) then
        total = total + tonumber(sz)
      end
    end
  end
  if total > 0 then
    return total
  end
  -- Fallback: estimate PCM size from composition geometry.
  local frames = tonumber(doc.frames) or 0
  local channels = tonumber(doc.channel_count) or 0
  if frames > 0 and channels > 0 then
    return frames * channels * 4
  end
  return 0
end

function Catalog:kept_totals(session)
  local docs = shared.docs_in_group(session, "keep")
  local bytes = 0
  for _, doc in ipairs(docs) do
    bytes = bytes + self:doc_size_bytes(doc)
  end
  return #docs, bytes
end

function Catalog:refresh_configure_previews()
  local function set_text(control, text)
    if control and control.text ~= text then
      control.text = text
    end
  end

  local lib = self:preview_path(self.library_root, "catalog.library_root")
  set_text(self.library_resolved, lib)

  local session = field.session.focused()
  local count, bytes = self:kept_totals(session)
  if count > 0 then
    set_text(
      self.selection_summary,
      string.format("%d composition(s)  ·  %s", count, self:format_bytes(bytes))
    )
  else
    set_text(self.selection_summary, "No documents in group `keep`")
  end
end

-- ── Destination planning ──────────────────────────────────────────────────────

--- Path-safe event segment for `{library}/{event}/…`.
--- Prefers `source.event.name` (enrich_media); falls back to the media
--- parent folder basename — needed after session reload because script
--- `source.*` bindings are in-memory only and enrich does not re-fire for
--- `.facomp` / `.fasession` opens.
function Catalog:sanitize_event_name(name)
  if not name or name == "" then
    return nil
  end
  if name == "." or name == ".." then
    return nil
  end
  if name:find("[/\\]") or name:find("^%s") or name:find("%s$") then
    return nil
  end
  return name
end

function Catalog:event_dir(doc)
  local ok, media_list = pcall(function()
    return doc.media
  end)
  if ok and media_list then
    for _, m in ipairs(media_list) do
      local ok2, bnd = pcall(function()
        return m:bindings("source.event")
      end)
      if ok2 and bnd then
        local ok3, name = pcall(function()
          return bnd.values and bnd.values.name
        end)
        local sanitized = ok3 and self:sanitize_event_name(name)
        if sanitized then
          return sanitized
        end
      end
      local ok4, parent_name = pcall(function()
        local parent = m.url and m.url.parent
        return parent and parent.basename
      end)
      local sanitized = ok4 and self:sanitize_event_name(parent_name)
      if sanitized then
        return sanitized
      end
    end
  end
  local ok5, doc_parent = pcall(function()
    local parent = doc.url and doc.url.parent
    return parent and parent.basename
  end)
  return ok5 and self:sanitize_event_name(doc_parent) or nil
end

function Catalog:layout_code(doc)
  local ok, layout = pcall(function()
    return doc.channel_layout
  end)
  if not ok or not layout then
    return nil
  end
  local registry = field.layouts.shared_registry()
  local ok2, items = pcall(function()
    return registry:items()
  end)
  if not ok2 then
    return nil
  end
  for _, item in ipairs(items or {}) do
    if item.name == layout then
      return item.code
    end
  end
  return nil
end

function Catalog:mixdown_chain(doc)
  local ok, chain = pcall(function()
    return doc.monitor_chain
  end)
  if ok and chain then
    for _, c in pairs(MIXDOWN_CHAINS) do
      if c == chain then
        return chain
      end
    end
  end
  local code = self:layout_code(doc)
  if code then
    return MIXDOWN_CHAINS[code]
  end
  return nil
end

function Catalog:sidecar_basename(basename, code, ext)
  local new_name = basename:gsub("%-" .. code .. "%.", "-st.")
  if new_name ~= basename then
    return new_name
  end
  local stem = basename:match("^(.-)%." .. ext .. "$")
  if stem then
    return stem .. "-st." .. ext
  end
  return basename .. "-st." .. ext
end

function Catalog:library_event_dir(doc, library_root)
  local event = self:event_dir(doc)
  if event then
    return shared.join_path(library_root, event)
  end
  return library_root
end

function Catalog:path_basename(path)
  if not path or path == "" then
    return nil
  end
  return path:match("([^/\\]+)$") or path
end

function Catalog:plan_outputs(doc, library_root, profile_name)
  local profile = field.exports.shared_registry():find(profile_name)
  if not profile then
    return nil, string.format("unknown export profile `%s`", profile_name)
  end

  local ext = profile.extension or "flac"
  local doc_name = doc.name or "take"
  local dir = self:library_event_dir(doc, library_root)
  local identity_path = shared.join_path(dir, doc_name .. "." .. ext)

  local outputs = {
    { path = identity_path, profile = profile_name },
  }

  local chain = self:mixdown_chain(doc)
  if chain then
    local code = self:layout_code(doc)
    local sidecar_name = code
        and self:sidecar_basename(doc_name .. "." .. ext, code, ext)
      or (doc_name .. "-st." .. ext)
    outputs[#outputs + 1] = {
      path = shared.join_path(dir, sidecar_name),
      profile = profile_name,
      chain = chain,
    }
  end

  return outputs, nil
end

--- Absolute `.facomp` path for a composition document, if it has one.
function Catalog:doc_facomp_path(doc, session)
  local base = self:session_base(session)
  local ok, url = pcall(function()
    return doc.url
  end)
  if not ok or not url then
    return nil
  end
  local path = self:path_from_url(url, base)
  if not path then
    return nil
  end
  if not path:lower():match("%.facomp$") then
    return nil
  end
  local exists_ok, exists = pcall(function()
    return field.fs.exists(path)
  end)
  if not (exists_ok and exists) then
    return nil
  end
  return path
end

--- Absolute focused `.fasession` path when present on disk.
function Catalog:session_file_path(session)
  local ok, url = pcall(function()
    return session.url
  end)
  if not ok or not url then
    return nil
  end
  local path = self:path_from_url(url, nil)
  if not path or not path:lower():match("%.fasession$") then
    return nil
  end
  local exists_ok, exists = pcall(function()
    return field.fs.exists(path)
  end)
  if not (exists_ok and exists) then
    return nil
  end
  return path
end

--- Plan copies of `.facomp` / `.fasession` into library event dirs beside renders.
--- Returns `{ { src, dest, kind, label }, … }`.
function Catalog:plan_project_copies(docs, library_root, session)
  local copies = {}
  local seen_dest = {}
  local event_dirs = {}

  for _, doc in ipairs(docs or {}) do
    local dir = self:library_event_dir(doc, library_root)
    event_dirs[self:norm_path(dir)] = dir

    local facomp = self:doc_facomp_path(doc, session)
    if facomp then
      local dest = shared.join_path(dir, self:path_basename(facomp))
      local key = self:norm_path(dest)
      if not seen_dest[key] then
        seen_dest[key] = true
        copies[#copies + 1] = {
          src = facomp,
          dest = dest,
          kind = "facomp",
          label = doc.name or self:path_basename(facomp),
        }
      end
    end
  end

  local session_path = self:session_file_path(session)
  if session_path then
    local session_name = self:path_basename(session_path)
    for _, dir in pairs(event_dirs) do
      local dest = shared.join_path(dir, session_name)
      local key = self:norm_path(dest)
      if not seen_dest[key] then
        seen_dest[key] = true
        copies[#copies + 1] = {
          src = session_path,
          dest = dest,
          kind = "fasession",
          label = session_name,
        }
      end
    end
  end

  table.sort(copies, function(a, b)
    return a.dest < b.dest
  end)
  return copies
end

function Catalog:copy_project_file(src, dest, label)
  if not src or not dest then
    return false
  end
  local ok_exists, exists = pcall(function()
    return field.fs.exists(dest)
  end)
  if ok_exists and exists then
    self:log_issue("warn", string.format("skip copy (exists): %s", dest))
    return false
  end
  local ok_parent, parent = pcall(function()
    return field.url.from_path(dest).parent
  end)
  if ok_parent and parent then
    local parent_path = parent:as_path()
    if parent_path and parent_path ~= "" and not field.fs.exists(parent_path) then
      field.fs.mkdir(parent_path)
    end
  end
  local ok, err = pcall(function()
    field.fs.copy(src, dest)
  end)
  if not ok then
    self:log_issue(
      "error",
      string.format("copy %s failed: %s → %s (%s)", label or src, src, dest, tostring(err))
    )
    return false
  end
  self.written_paths[#self.written_paths + 1] = dest
  local ok_st, st = pcall(function()
    return field.fs.stat(dest)
  end)
  if ok_st and st and st.size then
    self.written_bytes = (self.written_bytes or 0) + (tonumber(st.size) or 0)
  end
  field.log.info("catalog", string.format("copied %s → %s", src, dest))
  return true
end

function Catalog:preflight(docs, library_root, profile_name)
  local planned = {}
  local conflicts = {}
  local session = field.session.focused()

  local function note_path(p, label)
    local ok, exists = pcall(function()
      return field.fs.exists(p)
    end)
    if ok and exists then
      conflicts[#conflicts + 1] = string.format("exists: %s", p)
    end
    if planned[p] then
      conflicts[#conflicts + 1] = string.format(
        "collision: %s and %s → %s",
        planned[p],
        label,
        p
      )
    end
    planned[p] = label
  end

  for _, doc in ipairs(docs) do
    local outputs, err = self:plan_outputs(doc, library_root, profile_name)
    if not outputs then
      conflicts[#conflicts + 1] =
        string.format("%s: %s", doc.name or "?", err or "plan error")
    else
      for _, out in ipairs(outputs) do
        note_path(out.path, doc.name or "?")
      end
    end
  end

  for _, copy in ipairs(self:plan_project_copies(docs, library_root, session)) do
    note_path(copy.dest, copy.label or copy.kind)
  end

  if #conflicts > 0 then
    return false, table.concat(conflicts, "\n")
  end
  return true, nil
end

-- ── Staging cleanup candidates ────────────────────────────────────────────────

function Catalog:staging_prefix(staging_root)
  if not staging_root or staging_root == "" then
    return nil
  end
  local last = staging_root:sub(-1)
  if last == "/" or last == "\\" then
    return staging_root
  end
  local sep = staging_root:find("\\") and "\\" or "/"
  return staging_root .. sep
end

function Catalog:norm_path(path)
  return (path or ""):gsub("\\", "/"):gsub("/+$", "")
end

function Catalog:is_under_staging(path, staging_root, prefix)
  if not path or not staging_root then
    return false
  end
  local p = self:norm_path(path)
  local root = self:norm_path(staging_root)
  if p == root then
    return true
  end
  local pref = prefix and self:norm_path(prefix):gsub("/+$", "") .. "/" or (root .. "/")
  return p:sub(1, #pref) == pref
end

--- Directory containing the focused session file (for resolving relative URLs).
function Catalog:session_base(session)
  local ok, url = pcall(function()
    return session.url
  end)
  if not ok or not url then
    return nil
  end
  local ok2, parent = pcall(function()
    return url.parent and url.parent:as_path()
  end)
  if ok2 and parent and parent ~= "" then
    return parent
  end
  return nil
end

--- Make a path absolute using the session directory when the URL is relative
--- (session JSON stores media / .facomp urls relative to the .fasession).
function Catalog:absolute_path(path, session_base)
  if not path or path == "" then
    return nil
  end
  if path:sub(1, 1) == "/" or path:match("^%a:[/\\]") then
    return path
  end
  if session_base and session_base ~= "" then
    return shared.join_path(session_base, path)
  end
  return path
end

function Catalog:path_from_url(url, session_base)
  if not url then
    return nil
  end
  local ok, native = pcall(function()
    return url.native or url:as_path()
  end)
  if not ok or not native or native == "" then
    return nil
  end
  return self:absolute_path(native, session_base)
end

function Catalog:collect_cleanup_paths(session, staging_root)
  local prefix = self:staging_prefix(staging_root)
  local base = self:session_base(session)
  local seen = {}
  local paths = {}

  local function consider(path)
    path = self:absolute_path(path, base)
    if not path then
      return
    end
    local key = self:norm_path(path)
    if self:is_under_staging(path, staging_root, prefix) and not seen[key] then
      seen[key] = true
      paths[#paths + 1] = path
    end
  end

  -- Session file itself when it lives under staging.
  do
    local ok, url = pcall(function()
      return session.url
    end)
    if ok then
      consider(self:path_from_url(url, nil))
    end
  end

  for _, doc in ipairs(session.compositions or {}) do
    local ok, url = pcall(function()
      return doc.url
    end)
    if ok then
      consider(self:path_from_url(url, base))
    end
    local okm, media_list = pcall(function()
      return doc.media
    end)
    if okm and media_list then
      for _, m in ipairs(media_list) do
        local ok4, murl = pcall(function()
          return m.url
        end)
        if ok4 then
          consider(self:path_from_url(murl, base))
        end
      end
    end
  end

  table.sort(paths)
  return paths
end

function Catalog:parent_dir(path)
  local ok, url = pcall(function()
    return field.url.from_path(path)
  end)
  if not ok or not url or not url.parent then
    return nil
  end
  local parent = url.parent:as_path()
  if not parent or parent == "" then
    return nil
  end
  return parent
end

--- Parent dirs under staging (excluding staging_root) that would be empty after
--- the listed files are removed. Deepest paths first.
function Catalog:collect_empty_parent_dirs(paths, staging_root)
  if not staging_root or staging_root == "" or #paths == 0 then
    return {}
  end
  local prefix = self:staging_prefix(staging_root)
  local staging_norm = self:norm_path(staging_root)
  local cleanup_set = {}
  for _, path in ipairs(paths) do
    cleanup_set[self:norm_path(path)] = true
  end

  local seen = {}
  local candidates = {}
  for _, path in ipairs(paths) do
    local parent = self:parent_dir(path)
    while parent do
      local parent_norm = self:norm_path(parent)
      if parent_norm == staging_norm or parent_norm == "" then
        break
      end
      if not self:is_under_staging(parent, staging_root, prefix) then
        break
      end
      if not seen[parent_norm] then
        seen[parent_norm] = true
        candidates[#candidates + 1] = parent
      end
      local next_parent = self:parent_dir(parent)
      if not next_parent or self:norm_path(next_parent) == parent_norm then
        break
      end
      parent = next_parent
    end
  end

  local dirs = {}
  for _, dir in ipairs(candidates) do
    local ok, found = pcall(function()
      return field.fs.find_files(dir)
    end)
    if ok then
      local only_cleanup = true
      for _, rel in ipairs(found) do
        local abs = self:norm_path(shared.join_path(dir, rel))
        if not cleanup_set[abs] then
          only_cleanup = false
          break
        end
      end
      if only_cleanup then
        dirs[#dirs + 1] = dir
      end
    end
  end

  table.sort(dirs, function(a, b)
    return #self:norm_path(a) > #self:norm_path(b)
  end)
  return dirs
end

function Catalog:cleanup_summary_label(file_count, dir_count, staging)
  if not staging or staging == "" then
    return "No staging root"
  end
  if dir_count > 0 then
    local dir_word = dir_count == 1 and "directory" or "directories"
    return string.format(
      "%d file(s) under staging and %d %s",
      file_count,
      dir_count,
      dir_word
    )
  end
  return string.format("%d file(s) under staging", file_count)
end

-- ── Run: deferred polling (Ingest-shaped) ─────────────────────────────────────

function Catalog:begin_comp(doc, index, total, library_root, profile_name)
  self:set_comps_progress(
    index - 1,
    total,
    string.format("%d / %d composition(s)", index - 1, total)
  )
  self:set_current_comp(doc)
  self:set_render_progress(nil, "Planning…")

  local outputs, err = self:plan_outputs(doc, library_root, profile_name)
  if not outputs then
    self:log_issue("error", string.format("[%s] %s", doc.name or "?", err or "plan error"))
    return false
  end

  local job_ok, job_or_err = pcall(function()
    return doc:begin_render({ outputs = outputs })
  end)
  if not job_ok then
    self:log_issue("error", string.format("[%s] %s", doc.name or "?", tostring(job_or_err)))
    return false
  end

  self._job = job_or_err
  self._job_doc = doc
  self._job_index = index
  self._job_total = total
  self._job_outputs = outputs
  self:set_render_progress(0, "0%")
  return true
end

function Catalog:finish_current_comp()
  local job = self._job
  local doc = self._job_doc
  local index = self._job_index or 0
  local total = self._job_total or 0
  local name = (doc and doc.name) or "?"
  local library = self._catalog_library

  local result = {
    name = name,
    status = "ok",
    skipped = 0,
    failed = 0,
    written = {},
  }

  local render_err = job and job:error()
  if render_err then
    result.status = "failed"
    result.failed = 1
    self:log_issue("error", string.format("[%s] %s", name, tostring(render_err)))
  else
    for _, r in ipairs((job and job:results()) or {}) do
      if r.status == "skipped" then
        result.skipped = result.skipped + 1
        result.status = result.status == "ok" and "skipped" or result.status
        self:log_issue("warn", string.format("[%s] skipped: %s", name, r.path))
      elseif r.status == "failed" then
        result.failed = result.failed + 1
        result.status = "failed"
        self:log_issue(
          "error",
          string.format("[%s] failed: %s — %s", name, r.path, r.detail or "?")
        )
      else
        result.written[#result.written + 1] = r.path
        self.written_paths[#self.written_paths + 1] = r.path
        local ok, st = pcall(function()
          return field.fs.stat(r.path)
        end)
        if ok and st and st.size then
          self.written_bytes = (self.written_bytes or 0) + (tonumber(st.size) or 0)
        end
      end
    end
  end

  -- Copy the composition `.facomp` beside successful renders.
  if doc and library and #result.written > 0 then
    local dir = self:library_event_dir(doc, library)
    self._catalog_event_dirs = self._catalog_event_dirs or {}
    self._catalog_event_dirs[self:norm_path(dir)] = dir
    local session = field.session.focused()
    local facomp = self:doc_facomp_path(doc, session)
    if facomp then
      local dest = shared.join_path(dir, self:path_basename(facomp))
      if self:copy_project_file(facomp, dest, name .. ".facomp") then
        result.written[#result.written + 1] = dest
      end
    end
  end

  self.run_results[#self.run_results + 1] = result
  self._job = nil
  self._job_doc = nil
  self._job_index = nil
  self._job_total = nil
  self._job_outputs = nil

  self:set_render_progress(1, "100%")
  self:set_comps_progress(index, total, string.format("%d / %d composition(s)", index, total))
end

--- Copy the focused `.fasession` into each library event dir that received renders.
function Catalog:copy_session_to_library()
  local session = field.session.focused()
  local session_path = self:session_file_path(session)
  if not session_path then
    return
  end
  local dirs = self._catalog_event_dirs or {}
  local name = self:path_basename(session_path)
  for _, dir in pairs(dirs) do
    local dest = shared.join_path(dir, name)
    self:copy_project_file(session_path, dest, name)
  end
end

function Catalog:poll_job()
  local job = self._job
  if not job then
    return true
  end
  local done, total = job:progress()
  if total and total > 0 then
    local frac = done / total
    self:set_render_progress(
      frac,
      string.format("%d / %d frames", done, total)
    )
  else
    self:set_render_progress(nil, "Rendering…")
  end
  if not job.finished then
    return false
  end
  self:finish_current_comp()
  return true
end

function Catalog:run_catalog()
  if self.busy then
    return
  end
  local session = field.session.focused()
  self:persist(session)

  if self.continue_btn then
    self.continue_btn.enabled = false
  end
  if self.back_run_btn then
    self.back_run_btn.enabled = false
  end
  self.error_count = 0
  self.warning_count = 0
  self.run_results = {}
  self.written_paths = {}
  self.written_bytes = 0
  if self.log then
    self.log.text = ""
  end

  self:set_comps_progress(0, 0, "Starting…")
  self:set_render_progress(nil, "Waiting…")
  self:set_current_comp(nil)

  local library = self:resolved_library()
  if not library or library == "" then
    app:alert(
      "Catalog",
      "Set catalog.library_root / user.catalog.library_root (or a Library path) before running."
    )
    self:go("configure")
    return
  end

  local profile = field.exports.shared_registry():find(self.profile)
  if not profile then
    app:alert("Catalog", "Unknown export profile: " .. tostring(self.profile))
    self:go("configure")
    return
  end

  local docs = shared.docs_in_group(session, "keep")
  if #docs == 0 then
    app:alert("Catalog", "No documents in group `keep`. Finish Review first.")
    self:go("configure")
    return
  end

  local ok, conflict_msg = self:preflight(docs, library, self.profile)
  if not ok then
    for line in string.gmatch(conflict_msg or "", "[^\n]+") do
      self:log_issue("error", line)
    end
    app:alert("Catalog", "Preflight failed — see Issues on the Run page.")
    -- Stay on / show Run so the log is visible, then allow Back.
    self:go("run")
    if self.back_run_btn then
      self.back_run_btn.enabled = true
    end
    self:set_comps_progress(0, 0, "Preflight failed")
    self:set_render_progress(0, "—")
    return
  end

  if not field.fs.exists(library) then
    field.fs.mkdir(library)
  end

  self.busy = true
  self._catalog_docs = docs
  self._catalog_index = 1
  self._catalog_library = library
  self._catalog_profile = self.profile
  self._catalog_event_dirs = {}
  self._job = nil
  self:set_comps_progress(0, #docs)
  self:defer(function(wf)
    wf:process_next()
  end)
end

function Catalog:process_next()
  if self._job then
    if not self:poll_job() then
      self:defer(function(wf)
        wf:process_next()
      end)
    else
      self._catalog_index = (self._catalog_index or 1) + 1
      self:defer(function(wf)
        wf:process_next()
      end)
    end
    return
  end

  local docs = self._catalog_docs or {}
  local i = self._catalog_index or 1
  local total = #docs
  if i > total then
    self:copy_session_to_library()
    self.busy = false
    self._catalog_docs = nil
    self._catalog_index = nil
    self._catalog_library = nil
    self._catalog_profile = nil
    self._catalog_event_dirs = nil
    self:set_comps_progress(total, total, string.format("Done: %d composition(s)", total))
    self:set_render_progress(1, "Complete")
    self:set_current_comp(nil)
    if self.back_run_btn then
      self.back_run_btn.enabled = true
    end
    local issues = (self.error_count or 0) + (self.warning_count or 0)
    if issues == 0 then
      self:refresh_summary()
      if self.continue_btn then
        self.continue_btn.enabled = true
      end
      self:go("summary")
    else
      -- Stay on Run with logs; Continue stays disabled until a clean run.
      if self.continue_btn then
        self.continue_btn.enabled = false
      end
      self:log_issue(
        "warn",
        string.format(
          "Finished with %d error(s), %d warning(s) — fix issues before continuing.",
          self.error_count or 0,
          self.warning_count or 0
        )
      )
    end
    return
  end

  local started = self:begin_comp(
    docs[i],
    i,
    total,
    self._catalog_library,
    self._catalog_profile
  )
  if not started then
    self._catalog_index = i + 1
  end
  self:defer(function(wf)
    wf:process_next()
  end)
end

function Catalog:refresh_summary()
  local n_files = #self.written_paths
  local library = self:resolved_library() or "—"
  if self.summary_files then
    self.summary_files.text = string.format(
      "%d file(s) rendered  ·  %s",
      n_files,
      self:format_bytes(self.written_bytes or 0)
    )
  end
  if self.summary_dest then
    self.summary_dest.text = "Library: " .. library
  end
  if self.summary_list then
    local lines = {}
    for _, p in ipairs(self.written_paths) do
      lines[#lines + 1] = p
    end
    self.summary_list.text = table.concat(lines, "\n")
  end
end

function Catalog:refresh_cleanup_list()
  local session = field.session.focused()
  local staging = self:resolve_staging_root()
  local paths = self:collect_cleanup_paths(session, staging)
  local dirs = self:collect_empty_parent_dirs(paths, staging)
  self._cleanup_paths = paths
  self._cleanup_dirs = dirs
  if self.cleanup_list then
    if #paths == 0 then
      self.cleanup_list.text = staging ~= ""
          and ("No session-referenced files under:\n" .. staging)
        or "No staging root is configured; cleanup will be a no-op."
    else
      local lines = {}
      for _, path in ipairs(paths) do
        lines[#lines + 1] = path
      end
      if #dirs > 0 then
        lines[#lines + 1] = ""
        lines[#lines + 1] = "Empty directories after removal:"
        for _, dir in ipairs(dirs) do
          lines[#lines + 1] = dir
        end
      end
      self.cleanup_list.text = table.concat(lines, "\n")
    end
  end
  if self.cleanup_summary then
    self.cleanup_summary.text = self:cleanup_summary_label(#paths, #dirs, staging)
  end
end

function Catalog:do_cleanup(session)
  local staging = self:resolve_staging_root()
  -- Re-collect at confirm time so relative media URLs resolve against the
  -- live session directory and the .fasession is included when under staging.
  local paths = self:collect_cleanup_paths(session, staging)
  local dirs = self:collect_empty_parent_dirs(paths, staging)
  self._cleanup_paths = paths
  self._cleanup_dirs = dirs

  -- Force-close open documents (discard unsaved) so staging unlinks are not
  -- blocked by open handles or unsaved-changes dialogs.
  local docs = {}
  for _, doc in ipairs(session.compositions or {}) do
    docs[#docs + 1] = doc
  end
  for _, doc in ipairs(docs) do
    local ok, err = pcall(function()
      doc:close({ discard = true })
    end)
    if not ok then
      field.log.warn(
        "catalog",
        string.format("could not close document before cleanup: %s", tostring(err))
      )
    end
  end

  local removed = 0
  local failed = 0
  for _, path in ipairs(paths) do
    local ok, err = pcall(function()
      field.fs.remove(path)
    end)
    if ok then
      removed = removed + 1
      field.log.info("catalog", "removed staging: " .. path)
    else
      failed = failed + 1
      field.log.warn("catalog", string.format("could not remove %s: %s", path, tostring(err)))
    end
  end
  local removed_dirs = 0
  for _, dir in ipairs(dirs) do
    local ok, err = pcall(function()
      field.fs.remove(dir)
    end)
    if ok then
      removed_dirs = removed_dirs + 1
      field.log.info("catalog", "removed empty staging dir: " .. dir)
    else
      field.log.warn(
        "catalog",
        string.format("could not remove directory %s: %s", dir, tostring(err))
      )
    end
  end
  self._cleaned_up = true
  if removed_dirs > 0 then
    field.log.info(
      "catalog",
      string.format(
        "cleanup complete: %d file(s) and %d director%s removed (%d failed)",
        removed,
        removed_dirs,
        removed_dirs == 1 and "y" or "ies",
        failed
      )
    )
  else
    field.log.info(
      "catalog",
      string.format("cleanup complete: %d file(s) removed (%d failed)", removed, failed)
    )
  end
  return removed, removed_dirs, failed
end

-- ── Sheet ─────────────────────────────────────────────────────────────────────

function Catalog:build_sheet()
  self.comps_progress = field.ui.progress({
    id = "comps_progress",
    label = "Compositions",
    loading = true,
    text = "Idle",
  })
  self.current_comp = field.ui.message({
    id = "current_comp",
    text = "—",
  })
  self.render_progress = field.ui.progress({
    id = "render_progress",
    label = "Render",
    loading = true,
    text = "Idle",
  })
  self.log = field.ui.log({
    id = "log",
    label = "Issues",
    text = "",
  })

  self.back_run_btn = field.ui.button({
    id = "back_run",
    label = "Back",
    align = "left",
    enabled = false,
    action = function(_, workflow)
      if not workflow.busy then
        workflow:go("configure")
      end
    end,
  })
  self.continue_btn = field.ui.button({
    id = "continue",
    label = "Continue",
    enabled = false,
    action = function(_, workflow)
      workflow:refresh_summary()
      workflow:go("summary")
    end,
  })

  local library = field.ui.path_entry({
    id = "library_root",
    label = "Library",
    value = self.library_root or "",
    browse = "directory",
    action = function(ctrl, workflow)
      workflow.library_root = ctrl.value or ""
      workflow:refresh_configure_previews()
    end,
  })
  self.library_resolved = field.ui.message({
    id = "library_resolved",
    text = "—",
    color = app.theme.semantic.muted_foreground,
  })
  self.selection_summary = field.ui.message({
    id = "selection_summary",
    text = "—",
    color = app.theme.semantic.muted_foreground,
  })

  local profile_names = field.exports.shared_registry():names()
  local initial_profile = self.profile
  do
    local found = false
    for _, n in ipairs(profile_names) do
      if n == initial_profile then
        found = true
        break
      end
    end
    if not found and #profile_names > 0 then
      initial_profile = profile_names[1]
      self.profile = initial_profile
    end
  end
  local profile = field.ui.select({
    id = "profile",
    label = "Profile",
    value = initial_profile,
    choices = profile_names,
    action = function(ctrl, workflow)
      workflow.profile = ctrl.value or DEFAULT_PROFILE
    end,
  })

  self.summary_files = field.ui.message({
    id = "summary_files",
    text = "—",
  })
  self.summary_dest = field.ui.message({
    id = "summary_dest",
    text = "—",
    color = app.theme.semantic.muted_foreground,
  })
  self.summary_list = field.ui.log({
    id = "summary_list",
    label = "Rendered files",
    text = "",
  })

  self.cleanup_summary = field.ui.message({
    id = "cleanup_summary",
    text = "—",
  })
  self.cleanup_list = field.ui.log({
    id = "cleanup_list",
    label = "Files to remove",
    text = "",
  })

  self:refresh_configure_previews()

  local danger = app.theme.semantic.danger

  self:set_sheet({
    panes = {
      {
        id = "configure",
        name = "Configure",
        text = "Choose Library and an export profile, then Run. Paths may use ${…} variables.",
        controls = {
          library,
          self.library_resolved,
          self.selection_summary,
          profile,
        },
        buttons = {
          field.ui.button({
            id = "cancel",
            label = "Cancel",
            align = "left",
            action = function(_, workflow)
              workflow:close_sheet()
              field.workflow.cancel()
            end,
          }),
          field.ui.button({
            id = "run",
            label = "Run",
            action = function(_, workflow)
              workflow:go("run")
              workflow:defer(function(wf)
                wf:run_catalog()
              end)
            end,
          }),
        },
      },
      {
        id = "run",
        name = "Run",
        text = "Rendering kept compositions into the library. Queue above; current composition and render below.",
        controls = {
          self.comps_progress,
          self.current_comp,
          self.render_progress,
          self.log,
        },
        buttons = {
          self.back_run_btn,
          self.continue_btn,
        },
      },
      {
        id = "summary",
        name = "Summary",
        text = "Export complete. Finish closes the workflow, or continue to staging cleanup.",
        controls = {
          self.summary_files,
          self.summary_dest,
          self.summary_list,
        },
        buttons = {
          field.ui.button({
            id = "back_summary",
            label = "Back",
            align = "left",
            action = function(_, workflow)
              workflow:go("run")
            end,
          }),
          field.ui.button({
            id = "finish_summary",
            label = "Finish",
            action = function(_, workflow)
              workflow:close_sheet()
              field.workflow.finish()
            end,
          }),
          field.ui.button({
            id = "to_cleanup",
            label = "Cleanup",
            action = function(_, workflow)
              workflow:refresh_cleanup_list()
              workflow:go("cleanup")
            end,
          }),
        },
      },
      {
        id = "cleanup",
        name = "Cleanup",
        text = "Remove session-referenced files under staging (including the .fasession when it lives there). Paths outside staging are not touched.",
        controls = {
          self.cleanup_summary,
          self.cleanup_list,
        },
        buttons = {
          field.ui.button({
            id = "back_cleanup",
            label = "Back",
            align = "left",
            action = function(_, workflow)
              workflow:go("summary")
            end,
          }),
          field.ui.button({
            id = "cancel_cleanup",
            label = "Cancel",
            action = function(_, workflow)
              workflow:close_sheet()
              field.workflow.finish()
            end,
          }),
          field.ui.button({
            id = "confirm_cleanup",
            label = "Cleanup",
            color = danger,
            action = function(_, workflow)
              local ok, err = pcall(function()
                local sess = field.session.focused()
                -- Refresh list immediately before confirm so the dialog matches
                -- what will be deleted (includes session file + resolved media).
                workflow:refresh_cleanup_list()
                local n_files = #(workflow._cleanup_paths or {})
                local n_dirs = #(workflow._cleanup_dirs or {})
                if n_files == 0 then
                  field.log.warn("catalog", "cleanup: nothing under staging to remove")
                  workflow:close_sheet()
                  field.workflow.finish()
                  return
                end
                local detail = n_dirs > 0
                    and string.format(
                      "Remove %d session-referenced file(s) under staging and %d empty %s?\nThis cannot be undone.",
                      n_files,
                      n_dirs,
                      n_dirs == 1 and "directory" or "directories"
                    )
                  or string.format(
                    "Remove %d session-referenced file(s) under staging (including the session file when present)?\nThis cannot be undone.",
                    n_files
                  )
                -- FieldAssist shows a native dialog; destructive work runs in
                -- on_confirm (sync return is false once the dialog is open).
                app:confirm("Delete staging files?", detail, {
                  ok = "Cleanup",
                  danger = true,
                  on_confirm = function()
                    local cok, cerr = pcall(function()
                      workflow:do_cleanup(sess)
                      workflow:close_sheet()
                      field.workflow.finish()
                    end)
                    if not cok then
                      field.log.error(
                        "catalog",
                        "cleanup confirm failed: " .. tostring(cerr)
                      )
                    end
                  end,
                })
              end)
              if not ok then
                field.log.error("catalog", "cleanup action failed: " .. tostring(err))
              end
            end,
          }),
        },
      },
    },
  })
end

-- ── Lifecycle ─────────────────────────────────────────────────────────────────

function Catalog:start(_payload)
  local session = field.session.focused()
  self:restore(session)
  self:build_sheet()
  app:command("view.show-explorer")
end

function Catalog:suspend(session)
  self:persist(session)
  return true
end

function Catalog:resume(session)
  self:restore(session)
  self:build_sheet()
end

function Catalog:cancel(_session)
  field.log.info("catalog", "canceled")
end

function Catalog:finish(session)
  -- After staging cleanup the session / media files may already be gone;
  -- do not rewrite the deleted .fasession.
  if not self._cleaned_up then
    self:persist(session)
  end
  field.log.info("catalog", "finished")
end

field.workflow.declare(Catalog)

do
  local wf = setmetatable({}, { __index = Catalog })
  local function check_sidecar(input, code, ext, expected)
    local got = wf:sidecar_basename(input, code, ext)
    if got ~= expected then
      field.log.warn(
        "catalog",
        string.format(
          "sidecar_basename(%q, %q) = %q, expected %q",
          input,
          code,
          got,
          expected
        )
      )
    end
  end
  check_sidecar("take-ambix.flac", "ambix", "flac", "take-st.flac")
  check_sidecar("take-fuma.flac", "fuma", "flac", "take-st.flac")
  check_sidecar("take-ms.flac", "ms", "flac", "take-st.flac")
  check_sidecar("take.flac", "ambix", "flac", "take-st.flac")

  local function check_summary(files, dirs, staging, expected)
    local got = wf:cleanup_summary_label(files, dirs, staging)
    if got ~= expected then
      field.log.warn(
        "catalog",
        string.format(
          "cleanup_summary_label(%d, %d) = %q, expected %q",
          files,
          dirs,
          got,
          expected
        )
      )
    end
  end
  check_summary(3, 0, "/staging", "3 file(s) under staging")
  check_summary(3, 1, "/staging", "3 file(s) under staging and 1 directory")
  check_summary(5, 2, "/staging", "5 file(s) under staging and 2 directories")
  check_summary(0, 0, "", "No staging root")

  local function check_event(name, expected)
    local got = wf:sanitize_event_name(name)
    if got ~= expected then
      field.log.warn(
        "catalog",
        string.format(
          "sanitize_event_name(%q) = %q, expected %q",
          tostring(name),
          tostring(got),
          tostring(expected)
        )
      )
    end
  end
  check_event("26-09-27", "26-09-27")
  check_event("..", nil)
  check_event("a/b", nil)
end
