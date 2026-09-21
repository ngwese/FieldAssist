-- Spec example: Ingest workflow for the field-recording pipeline.
-- See docs/spec/SPEC-field-recording.md.
--
-- NOT embedded. NOT expected to run until intended host APIs ship:
--   app:confirm, app.url, app.fs, app.sqlite, media convert/tags,
--   background jobs, app:finish_workflow({ next = "…" })
--
-- Copy into the FieldAssist config directory next to init.lua.
-- Requires shared.lua in the same directory (loaded via dofile below).

local shared
do
  local info = debug.getinfo(1, "S")
  local src = info and info.source or ""
  local dir = src:match("^@(.*[/\\])") or ""
  shared = dofile(dir .. "shared.lua")
end

local Ingest = app:create_workflow({
  name = "ingest",
  display_name = "Ingest",
  description = "Copy/convert field recordings into backup and staging",
  scopes = { "drag-drop", "menu" },
  drop = { row = 3, priority = 1, color = app.theme.semantic.warning },
})

function Ingest:init()
  self.source = ""
  self.backup_root = ""
  self.staging_root = ""
  self.format = "flac"
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

function Ingest:build_toolbar()
  self.progress = app.ui.message({
    id = "progress",
    text = "Idle",
    color = app.theme.semantic.muted_foreground,
  })

  self:set_toolbar({
    app.ui.path_entry({
      id = "source",
      label = "Source",
      value = self.source or "",
      browse = "directory",
      action = function(ctrl, workflow)
        workflow.source = ctrl.value or ""
      end,
    }),
    app.ui.path_entry({
      id = "backup",
      label = "Backup",
      value = self.backup_root or "",
      browse = "directory",
      action = function(ctrl, workflow)
        workflow.backup_root = ctrl.value or ""
      end,
    }),
    app.ui.path_entry({
      id = "staging",
      label = "Staging",
      value = self.staging_root or "",
      browse = "directory",
      action = function(ctrl, workflow)
        workflow.staging_root = ctrl.value or ""
      end,
    }),
    app.ui.text_entry({
      id = "format",
      label = "Format",
      value = self.format or "flac",
      action = function(ctrl, workflow)
        workflow.format = (ctrl.value or "flac"):lower()
      end,
    }),
    app.ui.divider(),
    self.progress,
    app.ui.button({
      id = "run",
      label = "Run",
      align = "right",
      action = function(_, workflow)
        workflow:run_ingest()
      end,
    }),
    app.ui.button({
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

-- Intended background job body. Today :start is synchronous; when jobs land,
-- enqueue work and return from :start / Run immediately.
function Ingest:process_one(source_path, index, total)
  self:set_progress(string.format("Ingest %d / %d", index, total))

  -- local src = app.url(source_path)
  -- local staging_root = app.url(self.staging_root)
  -- app.fs.mkdir(staging_root, { recursive = true })

  if self.backup_root and self.backup_root ~= "" then
    -- local backup = app.url(self.backup_root):join(rel)
    -- app.fs.mkdir(backup.parent, { recursive = true })
    -- app.fs.copy(src, backup)
    -- assert(app.fs.checksum(src) == app.fs.checksum(backup))
    app:info("ingest", "backup (intended): " .. source_path)
  end

  -- Convert to preferred format in staging (not bit-exact):
  -- local dest = staging_root:join(rel):with_extension(self.format)
  -- app.fs.mkdir(dest.parent, { recursive = true })
  -- local tags = app.media_tags.canonical(app.media_tags.read(src))
  -- app.media_convert(src, dest, { format = self.format, tags = tags })
  -- Future C2PA on derivative only: app.c2pa.sign(dest, { ... })
  -- local sum = app.fs.checksum(dest)
  -- ledger insert: source_url, backup_url, staging_url, checksums, status=verified

  app:info("ingest", string.format("stage (intended) → %s: %s", self.format, source_path))

  local ok, doc = pcall(function()
    -- When convert exists, open the staging path instead of the source.
    return app.session:open(source_path)
  end)
  if ok and doc then
    doc.group = "todo"
  end
end

function Ingest:run_ingest()
  if self.busy then
    return
  end
  local session = app.session
  self:persist(session)

  if not self.staging_root or self.staging_root == "" then
    app:alert("Ingest", "Set a Staging directory before running.")
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

  -- Intended: open ledger once
  -- local db = app.sqlite.open(shared.ledger_path(self.staging_root))

  self.busy = true
  for i, path in ipairs(paths) do
    local ok, err = pcall(function()
      self:process_one(path, i, #paths)
    end)
    if not ok then
      app:error("ingest", tostring(err))
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
    app:info(
      "ingest",
      "source delete skipped (app:confirm not available); use Finish when ready"
    )
  end

  -- db:close()
end

function Ingest:finish_to_review()
  self:persist(app.session)
  -- Intended handoff:
  --   app:finish_workflow({ next = "review" })
  -- Until that ships, finish and prompt the user.
  app:finish_workflow()
  app:alert(
    "Ingest complete",
    "Start Review from the Workflow menu to continue the pipeline."
  )
end

function Ingest:start(payload)
  local session = app.session
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
  app:info("ingest", "canceled")
end

function Ingest:finish(session)
  self:persist(session)
  app:info("ingest", "finished")
end

app:declare_workflow(Ingest)
