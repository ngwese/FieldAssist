-- Spec example: Catalog workflow for the field-recording pipeline.
-- See docs/spec/SPEC-field-recording.md and SPEC-processing.md.
--
-- NOT embedded. NOT expected to run until intended host APIs ship:
--   app:confirm, app.url, app.fs, app.sqlite, composition:export / processing
--   chains, background jobs, field.workflow.finish({ next = "…" })
--
-- Copy into the FieldAssist config directory next to init.lua.
-- Requires shared.lua in the same directory.
-- Run after Review (example or built-in) has marked documents keep / drop.

local shared = field.include("shared.lua")

local Catalog = field.workflow.create({
  name = "catalog",
  display_name = "Catalog",
  description = "Export kept takes into the library and clean staging",
  scopes = { "menu" },
})

function Catalog:init()
  self.catalog_root = ""
  self.staging_root = ""
  self.format = "flac"
  self.busy = false
end

function Catalog:restore(session)
  local props = session.properties or {}
  self.catalog_root = self.catalog_root ~= "" and self.catalog_root
    or props.catalog_root
    or ""
  self.staging_root = self.staging_root ~= "" and self.staging_root
    or props.staging_root
    or ""
end

function Catalog:persist(session)
  shared.set_prop(session, "catalog_root", self.catalog_root)
  if self.staging_root and self.staging_root ~= "" then
    shared.set_prop(session, "staging_root", self.staging_root)
  end
end

function Catalog:set_progress(text)
  if self.progress then
    self.progress.text = text
  end
end

function Catalog:kept_docs(session)
  return shared.docs_in_group(session, "keep")
end

function Catalog:build_toolbar(session)
  self.progress = field.ui.message({
    id = "progress",
    text = "-",
    color = app.theme.semantic.muted_foreground,
  })

  local kept = #self:kept_docs(session)
  self.progress.text = string.format("%d kept", kept)

  self:set_toolbar({
    field.ui.path_entry({
      id = "catalog_root",
      label = "Library",
      value = self.catalog_root or "",
      browse = "directory",
      action = function(ctrl, workflow)
        workflow.catalog_root = ctrl.value or ""
      end,
    }),
    field.ui.text_entry({
      id = "format",
      label = "Format",
      value = self.format or "flac",
      action = function(ctrl, workflow)
        workflow.format = (ctrl.value or "flac"):lower()
      end,
    }),
    field.ui.divider(),
    self.progress,
    field.ui.button({
      id = "export",
      label = "Export kept",
      align = "right",
      action = function(_, workflow)
        workflow:run_catalog()
      end,
    }),
    field.ui.button({
      id = "finish",
      label = "Finish",
      align = "right",
      action = function(_, _)
        field.workflow.finish()
      end,
    }),
  })
end

function Catalog:export_one(doc, index, total)
  self:set_progress(string.format("Export %d / %d: %s", index, total, doc.name or "?"))

  -- Intended (SPEC-processing): apply bound or session-default chain, then encode.
  --
  --   local dest = app.url(self.catalog_root)
  --     :join((doc.name or "take") .. "." .. self.format)
  --   app.fs.mkdir(app.url(self.catalog_root), { recursive = true })
  --   doc:export({
  --     path = dest.native,
  --     format = self.format,
  --     -- chain snapshot captured at batch start
  --   })
  --
  -- Until that ships, File → Render remains the interactive single-file path.
  field.log.info(
    "catalog",
    string.format("export (intended) %s → %s", doc.name or doc.path or "?", self.catalog_root)
  )

  -- Update ledger: catalog_status = exported for this staging_url / document
end

function Catalog:cleanup_staging()
  if not self.staging_root or self.staging_root == "" then
    return
  end

  -- Intended:
  -- if not app:confirm(
  --   "Remove staging files?",
  --   "Delete converted copies under staging after successful catalog export?\n"
  --     .. "Backup is not touched.",
  --   { destructive = true }
  -- ) then
  --   return
  -- end
  -- for each ledger row with catalog_status == exported:
  --   app.fs.remove(staging_url)
  --   set catalog_status = staging_removed

  field.log.info(
    "catalog",
    "staging cleanup skipped (app:confirm / app.fs not available): " .. self.staging_root
  )
end

function Catalog:run_catalog()
  if self.busy then
    return
  end
  local session = field.session.shared()
  self:persist(session)

  if not self.catalog_root or self.catalog_root == "" then
    app:alert("Catalog", "Set a Library directory before exporting.")
    return
  end

  local docs = self:kept_docs(session)
  if #docs == 0 then
    app:alert("Catalog", "No documents in group `keep`. Finish Review first.")
    return
  end

  self.busy = true
  local failed = 0
  for i, doc in ipairs(docs) do
    local ok, err = pcall(function()
      self:export_one(doc, i, #docs)
    end)
    if not ok then
      failed = failed + 1
      field.log.error("catalog", tostring(err))
    end
  end
  self.busy = false

  if failed == 0 then
    self:set_progress(string.format("Exported %d", #docs))
    self:cleanup_staging()
  else
    self:set_progress(string.format("%d failed of %d", failed, #docs))
    field.log.warn("catalog", "Fix failures before removing staging.")
  end
end

function Catalog:start(_payload)
  local session = field.session.shared()
  self:restore(session)
  self:build_toolbar(session)
  app:command("view.show-explorer")
end

function Catalog:suspend(session)
  self:persist(session)
  return true
end

function Catalog:resume(session)
  self:restore(session)
  self:build_toolbar(session)
end

function Catalog:cancel(_session)
  field.log.info("catalog", "canceled")
end

function Catalog:finish(session)
  self:persist(session)
  field.log.info("catalog", "finished")
end

field.workflow.declare(Catalog)
