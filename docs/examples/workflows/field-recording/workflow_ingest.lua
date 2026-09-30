-- Spec example: Ingest workflow for the field-recording pipeline.
-- See docs/spec/SPEC-field-recording.md.
--
-- Modal sheet: Configure → Stage → Finish.
-- Configure gathers parameters; Run starts staging on the Stage pane.
-- Finish will later offer optional source cleanup (TBD); Finish ends the run.
--
-- Staging convert uses field.media.open + field.media.transcode. Still deferred:
-- app:confirm, bit-exact backup + checksums, app.sqlite ledger, background jobs
-- / :defer pump, field.workflow.finish({ next = "…" }), C2PA / media tags.
--
-- Staging defaults to user.ingest.staging_dir from variables.json (expanded).
-- Set e.g. ingest.staging_dir → ${env.HOME}/Music/FieldAssist/staging.
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
  -- ${user.…} expression. Empty → resolve user.ingest.staging_dir.
  self.staging_root = "${user.ingest.staging_dir}"
  self.profile = "FLAC (Source Equivalent)"
  self.queue = {}
  self.busy = false
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

function Ingest:set_progress(text, fraction)
  if self.progress then
    self.progress.text = text
    if fraction then
      self.progress.value = fraction * 100
      self.progress.loading = false
    end
  end
  if self.log then
    self.log:append(text)
  end
end

--- Expand `${user.…}` / bare `user.*`, else treat as a concrete path.
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
  return raw
end

function Ingest:resolved_staging()
  return self:resolve_path_or_var(self.staging_root, "user.ingest.staging_dir")
end

function Ingest:build_sheet()
  self.progress = field.ui.progress({
    id = "progress",
    label = "Progress",
    loading = true,
    text = "Idle",
  })
  self.log = field.ui.log({
    id = "log",
    label = "Log",
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
    end,
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

  self:set_sheet({
    panes = {
      {
        id = "configure",
        name = "Configure",
        text = "Choose a source folder and a staging profile, then Run.",
        controls = { source, profile },
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
              workflow:go("stage")
              -- Next UI frame so Stage paints before work starts.
              workflow:defer(function(wf)
                wf:run_ingest()
              end)
            end,
          }),
        },
      },
      {
        id = "stage",
        name = "Stage",
        text = "Staging media into backup and staging. Watch progress below.",
        controls = { self.progress, self.log },
        buttons = {
          field.ui.button({
            id = "back_stage",
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
              workflow:go("stage")
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
    for _, path in ipairs(shared.expand_media(self.source)) do
      paths[#paths + 1] = path
    end
  end
  return paths
end

-- Process one file per deferred frame so the Stage pane can paint progress.
-- When background jobs land, enqueue work and return immediately; enable
-- Continue from a job_done handler.
function Ingest:process_one(source_path, index, total, staging, profile)
  self:set_progress(string.format("Stage %d / %d", index, total), (index - 1) / total)

  local backup = self:resolve_path_or_var(self.backup_root, nil)
  if backup and backup ~= "" then
    -- Intended bit-exact backup (still deferred):
    -- local dest = field.url.from_path(backup):join(rel)
    -- field.fs.mkdir(dest.parent, { recursive = true })
    -- field.fs.copy(source_path, dest:as_path())
    field.log.info("ingest", "backup (intended): " .. source_path .. " → " .. backup)
  end

  local src = field.media.open(source_path)
  local dest = field.url
    .from_path(staging)
    :join(src.url.stem .. "." .. profile.extension)
    :as_path()

  field.log.info(
    "ingest",
    string.format("transcode → %s (%s)", dest, profile.name or self.profile)
  )

  field.media.transcode(src, dest, profile, function(done, frames)
    if frames <= 0 then
      return
    end
    local file_frac = done / frames
    local overall = ((index - 1) + file_frac) / total
    self:set_progress(string.format(
      "Stage %d / %d  (%d%%)",
      index,
      total,
      math.floor(file_frac * 100)
    ), overall)
  end)

  -- Future: tags / C2PA on the derivative only; ledger row with checksums.

  local ok, doc = pcall(function()
    return field.session.focused():open(dest)
  end)
  if ok and doc then
    doc.group = "todo"
  else
    error(ok and "session:open returned nil" or tostring(doc), 0)
  end
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
  if self.progress then
    self.progress.loading = true
    self.progress.text = "Starting…"
  end

  local staging = self:resolved_staging()
  if not staging or staging == "" then
    app:alert(
      "Ingest",
      "Set user.ingest.staging_dir in variables.json (or a Staging path) before running."
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
  self:set_progress(string.format("Stage 0 / %d", #paths), 0)
  self:defer(function(wf)
    wf:process_next()
  end)
end

function Ingest:process_next()
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
    self:set_progress(string.format("Done: %d file(s)", total), 1)
    if self.continue_btn then
      self.continue_btn.enabled = true
    end
    return
  end

  local path = paths[i]
  local ok, err = pcall(function()
    self:process_one(path, i, total, self._ingest_staging, self._ingest_profile)
  end)
  if not ok then
    field.log.error("ingest", tostring(err))
    if self.log then
      self.log:append("error: " .. tostring(err))
    end
  end
  self._ingest_index = i + 1
  self:set_progress(string.format("Stage %d / %d", i, total), i / total)
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
