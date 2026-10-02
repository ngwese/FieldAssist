-- Spec example: Ingest workflow for the field-recording pipeline.
-- See docs/spec/SPEC-field-recording.md.
--
-- Modal sheet: Configure → Run → Finish.
-- Configure gathers parameters; Run starts staging on the Run pane.
-- Run shows file-level and per-file transcode progress; the log is for
-- errors/warnings only. Finish will later offer optional source cleanup
-- (TBD); Finish ends the run.
--
-- Staging convert uses field.media.open + field.media.transcode. Still deferred:
-- app:confirm, bit-exact backup + checksums, app.sqlite ledger, background jobs
-- / :defer pump, field.workflow.finish({ next = "…" }), C2PA / media tags.
--
-- Staging / Backup default to ${user.ingest.staging_root} /
-- ${user.ingest.backup_root} (expanded); Configure shows path fields with
-- muted resolved previews (input file count/size under Source; free space
-- under Staging / Backup). Paths may use ${…} templates.
--
-- Copy into the FieldAssist config directory next to init.lua.
-- Requires shared.lua in the same directory (via field.include).

local shared = field.include("shared.lua")

local Ingest = field.workflow.create({
  name = "ingest",
  display_name = "Ingest",
  description = "Copy/convert field recordings into backup and staging",
  scopes = { "drag-drop", "menu" },
  drop = { row = 3, priority = 1, color = app.theme.semantic.warning },
})

function Ingest:init()
  self.source = ""
  self.backup_root = "${user.ingest.backup_root}"
  -- Session prop name stays staging_root; value may be a concrete path or a
  -- ${user.…} expression. Empty → resolve user.ingest.staging_root.
  self.staging_root = "${user.ingest.staging_root}"
  self.profile = "FLAC (Source Equivalent)"
  self.queue = {}
  self.busy = false
  self.error_count = 0
end

function Ingest:restore(session)
  self.source = self.source ~= "" and self.source
    or shared.get_prop(session, "ingest_source")
  self.backup_root = self.backup_root ~= "" and self.backup_root
    or shared.get_prop(session, "backup_root")
  self.staging_root = self.staging_root ~= "" and self.staging_root
    or shared.get_prop(session, "staging_root")
end

function Ingest:persist(session)
  shared.set_prop(session, "ingest_source", self.source)
  shared.set_prop(session, "backup_root", self.backup_root)
  shared.set_prop(session, "staging_root", self.staging_root)
end

function Ingest:format_bytes(n)
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

function Ingest:format_duration(seconds)
  seconds = tonumber(seconds) or 0
  if seconds < 0 then
    seconds = 0
  end
  local total = math.floor(seconds + 0.5)
  local h = math.floor(total / 3600)
  local m = math.floor((total % 3600) / 60)
  local s = total % 60
  if h > 0 then
    return string.format("%d:%02d:%02d", h, m, s)
  end
  return string.format("%d:%02d", m, s)
end

function Ingest:set_files_progress(done_files, total_files, caption)
  if not self.files_progress then
    return
  end
  total_files = math.max(tonumber(total_files) or 0, 0)
  done_files = math.max(tonumber(done_files) or 0, 0)
  if total_files <= 0 then
    self.files_progress.loading = true
    self.files_progress.text = caption or "Idle"
    return
  end
  self.files_progress.loading = false
  self.files_progress.value = math.min(100, (done_files / total_files) * 100)
  self.files_progress.text = caption
    or string.format("%d / %d file(s)", done_files, total_files)
end

function Ingest:set_file_progress(fraction, caption)
  if not self.file_progress then
    return
  end
  if fraction == nil then
    self.file_progress.loading = true
    self.file_progress.text = caption or "…"
    return
  end
  self.file_progress.loading = false
  self.file_progress.value = math.max(0, math.min(100, (tonumber(fraction) or 0) * 100))
  self.file_progress.text = caption
    or string.format("%d%%", math.floor(self.file_progress.value + 0.5))
end

function Ingest:set_current_file(media)
  if not self.current_file then
    return
  end
  if not media then
    self.current_file.text = "—"
    return
  end
  local name = media.basename or (media.url and media.url.basename) or "?"
  local size = self:format_bytes(media.size_bytes)
  local length = self:format_duration(media.duration)
  self.current_file.text = string.format("%s  ·  %s  ·  %s", name, size, length)
end

function Ingest:log_issue(level, message)
  field.log[level]("ingest", message)
  if level == "error" then
    self.error_count = (self.error_count or 0) + 1
  end
  if self.log then
    self.log:append(string.format("%s: %s", level, message))
  end
end

--- Expand path templates: whole `${name}` / bare `user.*`, inline `${…}`,
--- or a concrete path. Empty `raw` uses optional `fallback_name`.
function Ingest:resolve_path_or_var(raw, fallback_name)
  if not raw or raw == "" then
    raw = fallback_name
  end
  if not raw or raw == "" then
    return nil
  end
  local resolver = field.variables.user():variable_resolver()
  local name = raw:match("^%$%{(.+)%}$")
  if name then
    return resolver:resolve(name, true)
  end
  if raw:match("^user%.") then
    return resolver:resolve(raw, true)
  end
  if raw:find("${", 1, true) then
    return resolver:expand(raw)
  end
  return raw
end

function Ingest:resolved_staging()
  return self:resolve_path_or_var(self.staging_root, "user.ingest.staging_root")
end

--- Soft-resolved path for muted configure previews (`—` when empty).
function Ingest:preview_path(raw, fallback_name)
  local resolved = self:resolve_path_or_var(raw, fallback_name)
  if not resolved or resolved == "" then
    return "—"
  end
  return resolved
end

--- Sum media file sizes under a resolved source path (`0, 0` when empty).
function Ingest:source_input_totals(resolved)
  if not resolved or resolved == "" or resolved == "—" then
    return 0, 0
  end
  local paths = shared.expand_media(resolved)
  local total_bytes = 0
  local count = 0
  for _, path in ipairs(paths) do
    local ok, stat = pcall(function()
      return field.fs.stat(path)
    end)
    if ok and stat and not stat.is_dir then
      total_bytes = total_bytes + (tonumber(stat.size) or 0)
      count = count + 1
    end
  end
  return count, total_bytes
end

--- Free space suffix for a resolved staging/backup path (`nil` on failure).
function Ingest:free_space_label(resolved)
  if not resolved or resolved == "" or resolved == "—" then
    return nil
  end
  local ok, bytes = pcall(function()
    return field.fs.available_space(resolved)
  end)
  if not ok or bytes == nil then
    return nil
  end
  return self:format_bytes(bytes) .. " free"
end

function Ingest:refresh_path_previews()
  local function set_text(control, text)
    if not control then
      return
    end
    if control.text ~= text then
      control.text = text
    end
  end

  local source_path = self:preview_path(self.source, nil)
  if source_path == "—" then
    set_text(self.source_resolved, "—")
  else
    local count, bytes = self:source_input_totals(source_path)
    if count > 0 then
      set_text(
        self.source_resolved,
        string.format(
          "%s  ·  %d file(s)  ·  %s",
          source_path,
          count,
          self:format_bytes(bytes)
        )
      )
    else
      set_text(self.source_resolved, source_path)
    end
  end

  local staging_path = self:preview_path(self.staging_root, "user.ingest.staging_root")
  local staging_free = self:free_space_label(staging_path)
  if staging_path == "—" then
    set_text(self.staging_resolved, "—")
  elseif staging_free then
    set_text(self.staging_resolved, staging_path .. "  ·  " .. staging_free)
  else
    set_text(self.staging_resolved, staging_path)
  end

  local backup_path = self:preview_path(self.backup_root, nil)
  local backup_free = self:free_space_label(backup_path)
  if backup_path == "—" then
    set_text(self.backup_resolved, "—")
  elseif backup_free then
    set_text(self.backup_resolved, backup_path .. "  ·  " .. backup_free)
  else
    set_text(self.backup_resolved, backup_path)
  end
end

function Ingest:build_sheet()
  self.files_progress = field.ui.progress({
    id = "files_progress",
    label = "Files",
    loading = true,
    text = "Idle",
  })
  self.current_file = field.ui.message({
    id = "current_file",
    text = "—",
  })
  self.file_progress = field.ui.progress({
    id = "file_progress",
    label = "Transcode",
    loading = true,
    text = "Idle",
  })
  self.log = field.ui.log({
    id = "log",
    label = "Issues",
    text = "",
  })
  self.continue_btn = field.ui.button({
    id = "continue",
    label = "Continue",
    enabled = false,
    action = function(_, workflow)
      workflow:go("finish")
    end,
  })

  local source = field.ui.path_entry({
    id = "source",
    label = "Source",
    value = self.source or "",
    browse = "directory",
    action = function(ctrl, workflow)
      workflow.source = ctrl.value or ""
      workflow:refresh_path_previews()
    end,
  })
  self.source_resolved = field.ui.message({
    id = "source_resolved",
    text = "—",
  })
  local staging = field.ui.path_entry({
    id = "staging",
    label = "Staging",
    value = self.staging_root or "",
    browse = "directory",
    action = function(ctrl, workflow)
      workflow.staging_root = ctrl.value or ""
      workflow:refresh_path_previews()
    end,
  })
  self.staging_resolved = field.ui.message({
    id = "staging_resolved",
    text = "—",
  })
  local backup = field.ui.path_entry({
    id = "backup",
    label = "Backup",
    value = self.backup_root or "",
    browse = "directory",
    action = function(ctrl, workflow)
      workflow.backup_root = ctrl.value or ""
      workflow:refresh_path_previews()
    end,
  })
  self.backup_resolved = field.ui.message({
    id = "backup_resolved",
    text = "—",
  })
  local profile = field.ui.select({
    id = "profile",
    label = "Profile",
    value = self.profile or "",
    choices = field.exports.shared_registry():names(),
    action = function(ctrl, workflow)
      workflow.profile = ctrl.value or ""
    end,
  })

  self:refresh_path_previews()

  self:set_sheet({
    panes = {
      {
        id = "configure",
        name = "Configure",
        text = "Choose Source, Staging, Backup, and a staging profile, then Run. Paths may use ${…} variables.",
        controls = {
          source,
          self.source_resolved,
          staging,
          self.staging_resolved,
          backup,
          self.backup_resolved,
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
              -- Next UI frame so Run paints before work starts.
              workflow:defer(function(wf)
                wf:run_ingest()
              end)
            end,
          }),
        },
      },
      {
        id = "run",
        name = "Run",
        text = "Staging media into backup and staging. File queue above; current file and transcode below.",
        controls = {
          self.files_progress,
          self.current_file,
          self.file_progress,
          self.log,
        },
        buttons = {
          field.ui.button({
            id = "back_run",
            label = "Back",
            align = "left",
            enabled = false,
            action = function(_, workflow)
              if not workflow.busy then
                workflow:go("configure")
              end
            end,
          }),
          self.continue_btn,
        },
      },
      {
        id = "finish",
        name = "Finish",
        text = "Staging is complete. Optional source cleanup controls will live here (TBD). Finish closes the sheet and ends the workflow.",
        -- TBD: toggles / confirms to remove source files and parent directory.
        controls = {},
        buttons = {
          field.ui.button({
            id = "back_finish",
            label = "Back",
            align = "left",
            action = function(_, workflow)
              workflow:go("run")
            end,
          }),
          field.ui.button({
            id = "finish",
            label = "Finish",
            action = function(_, workflow)
              workflow:end_ingest()
            end,
          }),
        },
      },
    },
  })
end

function Ingest:collect_sources(payload)
  local paths = {}
  local incoming = payload.paths or {}
  if #incoming > 0 then
    -- A drop always defines this run's source (override any restored path).
    self.source = incoming[1]
    for _, item in ipairs(incoming) do
      if not shared.is_session_path(item) then
        for _, path in ipairs(shared.expand_media(item)) do
          paths[#paths + 1] = path
        end
      end
    end
  elseif self.source ~= "" then
    local resolved = self:resolve_path_or_var(self.source, nil)
    if resolved and resolved ~= "" then
      for _, path in ipairs(shared.expand_media(resolved)) do
        paths[#paths + 1] = path
      end
    end
  end
  return paths
end

-- Process one file per deferred frame so the Run pane can paint progress.
-- Transcode runs on a background job; `:defer` polls it so the File bar
-- updates without blocking the UI.
function Ingest:begin_file(source_path, index, total, staging, profile)
  self:set_files_progress(
    index - 1,
    total,
    string.format("%d / %d file(s)", index - 1, total)
  )
  self:set_file_progress(nil, "Opening…")
  self:set_current_file(nil)

  local backup = self:resolve_path_or_var(self.backup_root, nil)
  if backup and backup ~= "" then
    -- Intended bit-exact backup (still deferred):
    -- local dest = field.url.from_path(backup):join(rel)
    -- field.fs.mkdir(dest.parent, { recursive = true })
    -- field.fs.copy(source_path, dest:as_path())
    field.log.info("ingest", "backup (intended): " .. source_path .. " → " .. backup)
  end

  local ok, src_or_err = pcall(field.media.open, source_path)
  if not ok then
    self:log_issue("error", tostring(src_or_err))
    return false
  end
  local src = src_or_err
  self:set_current_file(src)
  self:set_file_progress(0, "0%")

  local dest = field.url
    .from_path(staging)
    :join(src.url.stem .. "." .. profile.extension)
    :as_path()

  field.log.info(
    "ingest",
    string.format("transcode → %s (%s)", dest, profile.name or self.profile)
  )

  local job_ok, job_or_err = pcall(field.media.begin_transcode, src, dest, profile)
  if not job_ok then
    self:log_issue("error", tostring(job_or_err))
    return false
  end

  self._job = job_or_err
  self._job_dest = dest
  self._job_index = index
  self._job_total = total
  return true
end

function Ingest:finish_current_file()
  local dest = self._job_dest
  self._job = nil
  self._job_dest = nil
  local index = self._job_index or 0
  local total = self._job_total or 0
  self._job_index = nil
  self._job_total = nil

  -- Future: tags / C2PA on the derivative only; ledger row with checksums.

  local ok, doc = pcall(function()
    return field.session.focused():open(dest)
  end)
  if ok and doc then
    doc.group = "todo"
  else
    self:log_issue(
      "error",
      ok and "session:open returned nil" or tostring(doc)
    )
  end

  self:set_file_progress(1, "100%")
  self:set_files_progress(
    index,
    total,
    string.format("%d / %d file(s)", index, total)
  )
end

function Ingest:poll_job()
  local job = self._job
  if not job then
    return true
  end
  local done, total = job:progress()
  if total and total > 0 then
    local frac = done / total
    self:set_file_progress(frac, string.format("%d%%", math.floor(frac * 100)))
  else
    self:set_file_progress(nil, "Transcoding…")
  end
  if not job.finished then
    return false
  end
  local err = job:error()
  if err then
    self:log_issue("error", tostring(err))
    self._job = nil
    self._job_dest = nil
    self._job_index = nil
    self._job_total = nil
    return true
  end
  self:finish_current_file()
  return true
end

function Ingest:run_ingest()
  if self.busy then
    return
  end
  local session = field.session.focused()
  self:persist(session)

  if self.continue_btn then
    self.continue_btn.enabled = false
  end
  self.error_count = 0
  self:set_files_progress(0, 0, "Starting…")
  self:set_file_progress(nil, "Waiting…")
  self:set_current_file(nil)

  local staging = self:resolved_staging()
  if not staging or staging == "" then
    app:alert(
      "Ingest",
      "Set user.ingest.staging_root in variables.json (or a Staging path) before running."
    )
    self:go("configure")
    return
  end

  local profile = field.exports.shared_registry():find(self.profile)
  if not profile then
    app:alert("Ingest", "Unknown export profile: " .. tostring(self.profile))
    self:go("configure")
    return
  end

  local paths = self.queue
  if not paths or #paths == 0 then
    paths = self:collect_sources({ paths = {} })
  end
  if #paths == 0 then
    app:alert("Ingest", "No media paths to ingest.")
    self:go("configure")
    return
  end

  if not field.fs.exists(staging) then
    field.fs.mkdir(staging)
  end

  -- Intended: open ledger once
  -- local db = app.sqlite.open(shared.ledger_path(staging))

  self.busy = true
  self._ingest_paths = paths
  self._ingest_index = 1
  self._ingest_staging = staging
  self._ingest_profile = profile
  self._job = nil
  self:set_files_progress(0, #paths)
  self:defer(function(wf)
    wf:process_next()
  end)
end

function Ingest:process_next()
  -- Continue polling an in-flight background transcode.
  if self._job then
    if not self:poll_job() then
      self:defer(function(wf)
        wf:process_next()
      end)
    else
      self._ingest_index = (self._ingest_index or 1) + 1
      self:defer(function(wf)
        wf:process_next()
      end)
    end
    return
  end

  local paths = self._ingest_paths or {}
  local i = self._ingest_index or 1
  local total = #paths
  if i > total then
    self.busy = false
    self.queue = {}
    self._ingest_paths = nil
    self._ingest_index = nil
    self._ingest_staging = nil
    self._ingest_profile = nil
    self:set_files_progress(total, total, string.format("Done: %d file(s)", total))
    self:set_file_progress(1, "Complete")
    self:set_current_file(nil)
    if self.continue_btn then
      self.continue_btn.enabled = true
    end
    if (self.error_count or 0) == 0 then
      self:go("finish")
    end
    return
  end

  local started = self:begin_file(
    paths[i],
    i,
    total,
    self._ingest_staging,
    self._ingest_profile
  )
  if not started then
    self._ingest_index = i + 1
  end
  self:defer(function(wf)
    wf:process_next()
  end)
end

function Ingest:end_ingest()
  self:persist(field.session.focused())
  -- Intended handoff after Finish:
  --   field.workflow.finish({ next = "review" })
  self:close_sheet()
  field.workflow.finish()
end

function Ingest:start(payload)
  local session = field.session.focused()
  self:restore(session)
  -- Drop paths override restored source inside collect_sources.
  self.queue = self:collect_sources(payload or {})
  self:build_sheet()
end

function Ingest:suspend(session)
  self:persist(session)
  return true
end

function Ingest:resume(session)
  self:restore(session)
  self:build_sheet()
end

function Ingest:cancel(_session)
  field.log.info("ingest", "canceled")
end

function Ingest:finish(session)
  self:persist(session)
  field.log.info("ingest", "finished")
end

field.workflow.declare(Ingest)
