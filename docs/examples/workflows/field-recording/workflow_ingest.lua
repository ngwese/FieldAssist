-- Spec example: Ingest workflow for the field-recording pipeline.
-- See docs/spec/SPEC-field-recording.md.
--
-- Staging convert uses field.media.open + field.media.transcode (ships today).
-- Still deferred: app:confirm, bit-exact backup + checksums, app.sqlite ledger,
-- background jobs, field.workflow.finish({ next = "…" }), C2PA / media tags.
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

function Ingest:set_progress(text)
  if self.progress then
    self.progress.text = text
  end
  -- Intended determinate progress:
  --   self:set_progress_state({ label = text, fraction = n / total, current = n, total = total })
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

function Ingest:build_toolbar()
  self.progress = field.ui.message({
    id = "progress",
    text = "Idle",
    color = app.theme.semantic.muted_foreground,
  })

  self:set_toolbar({
    field.ui.path_entry({
      id = "source",
      label = "Source",
      value = self.source or "",
      browse = "directory",
      action = function(ctrl, workflow)
        workflow.source = ctrl.value or ""
      end,
    }),
    -- field.ui.path_entry({
    --   id = "backup",
    --   label = "Backup",
    --   value = self.backup_root or "",
    --   browse = "directory",
    --   action = function(ctrl, workflow)
    --     workflow.backup_root = ctrl.value or ""
    --   end,
    -- }),
    -- field.ui.path_entry({
    --   id = "staging",
    --   label = "Staging",
    --   value = self.staging_root or "",
    --   browse = "directory",
    --   action = function(ctrl, workflow)
    --     workflow.staging_root = ctrl.value or ""
    --   end,
    -- }),

    field.ui.select({
      id = "profile",
      label = "Profile",
      value = self.profile or "",
      choices = field.exports.shared_registry():items(),
      action = function(ctrl, workflow)
        workflow.profile = ctrl.value or ""
      end,
    }),
    field.ui.divider(),
    self.progress,
    field.ui.button({
      id = "run",
      label = "Run",
      align = "right",
      action = function(_, workflow)
        workflow:run_ingest()
      end,
    }),
    field.ui.button({
      id = "finish",
      label = "Finish → Review",
      align = "right",
      action = function(_, workflow)
        workflow:finish_to_review()
      end,
    }),
  })
end

function Ingest:collect_sources(payload)
  local paths = {}
  local incoming = payload.paths or {}
  if #incoming > 0 then
    for _, item in ipairs(incoming) do
      if not shared.is_session_path(item) then
        for _, path in ipairs(shared.expand_media(item)) do
          paths[#paths + 1] = path
        end
      end
    end
    if self.source == "" and #incoming > 0 then
      self.source = incoming[1]
    end
  elseif self.source ~= "" then
    for _, path in ipairs(shared.expand_media(self.source)) do
      paths[#paths + 1] = path
    end
  end
  return paths
end

-- Today :start / Run are synchronous. When background jobs land, enqueue work
-- and return from :start / Run immediately.
function Ingest:process_one(source_path, index, total, staging, profile)
  self:set_progress(string.format("Ingest %d / %d", index, total))

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
    self:set_progress(string.format(
      "Ingest %d / %d  (%d%%)",
      index,
      total,
      math.floor((done * 100) / frames)
    ))
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

  local staging = self:resolved_staging()
  if not staging or staging == "" then
    app:alert(
      "Ingest",
      "Set user.ingest.staging_dir in variables.json (or a Staging path) before running."
    )
    return
  end

  local profile = field.exports.shared_registry():find(self.profile)
  if not profile then
    app:alert("Ingest", "Unknown export profile: " .. tostring(self.profile))
    return
  end

  local paths = self.queue
  if not paths or #paths == 0 then
    paths = self:collect_sources({ paths = {} })
  end
  if #paths == 0 then
    app:alert("Ingest", "No media paths to ingest.")
    return
  end

  if not field.fs.exists(staging) then
    field.fs.mkdir(staging)
  end

  -- Intended: open ledger once
  -- local db = app.sqlite.open(shared.ledger_path(staging))

  self.busy = true
  for i, path in ipairs(paths) do
    local ok, err = pcall(function()
      self:process_one(path, i, #paths, staging, profile)
    end)
    if not ok then
      field.log.error("ingest", tostring(err))
    end
  end
  self.busy = false
  self.queue = {}
  self:set_progress(string.format("Done: %d file(s)", #paths))

  if self.source and self.source ~= "" then
    -- Intended confirm before wiping the recorder:
    -- if app:confirm(
    --   "Remove source media?",
    --   "Delete verified files from the recorder?",
    --   { destructive = true }
    -- ) then
    --   for each verified source: app.fs.remove(src)
    -- end
    field.log.info(
      "ingest",
      "source delete skipped (app:confirm not available); use Finish when ready"
    )
  end

  -- db:close()
end

function Ingest:finish_to_review()
  self:persist(field.session.focused())
  -- Intended handoff:
  --   field.workflow.finish({ next = "review" })
  -- Until that ships, finish and prompt the user.
  field.workflow.finish()
  app:alert(
    "Ingest complete",
    "Start Review from the Workflow menu to continue the pipeline."
  )
end

function Ingest:start(payload)
  local session = field.session.focused()
  self:restore(session)
  self.queue = self:collect_sources(payload or {})
  if #self.queue > 0 and (not self.source or self.source == "") then
    self.source = self.queue[1]
  end
  self:build_toolbar()
  self:set_progress(
    #self.queue > 0 and string.format("%d path(s) ready", #self.queue) or "Set paths, then Run"
  )
  if payload and payload.scope == "drag-drop" and #self.queue > 0 then
    -- Auto-run on drop once background jobs exist; for the sketch, wait for Run
    -- so the user can confirm Backup / Staging first.
  end
end

function Ingest:suspend(session)
  self:persist(session)
  return true
end

function Ingest:resume(session)
  self:restore(session)
  self:build_toolbar()
  self:set_progress("Resumed")
end

function Ingest:cancel(_session)
  field.log.info("ingest", "canceled")
end

function Ingest:finish(session)
  self:persist(session)
  field.log.info("ingest", "finished")
end

field.workflow.declare(Ingest)
