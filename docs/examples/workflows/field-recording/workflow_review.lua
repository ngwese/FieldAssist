-- Spec example: Review workflow for the field-recording pipeline.
-- Adapted from crates/field-assist/assets/workflow_review.lua to match the
-- Ingest / Catalog example structure (shared.lua, handoff). Output directory
-- belongs on Catalog (`catalog_root`), not on this stage.
-- See docs/spec/SPEC-field-recording.md.
--
-- Copy into the FieldAssist config directory next to init.lua to override the
-- built-in `review` name. Requires shared.lua in the same directory.
-- Runs on today's host (no future APIs required for Keep / Drop).

local shared = field.include("shared.lua")

local Review = field.workflow.create({
  name = "review",
  display_name = "Review",
  description = "Review session documents",
  scopes = { "drag-drop", "menu" },
  drop = { row = 2, priority = 1, color = app.theme.semantic.info },
})

function Review:set_review_playback(on)
  shared.ensure_command_flag("transport.loop", on)
  shared.ensure_command_flag("transport.preview", on)
end

function Review:progress_text(session)
  local kept = session:group_count("keep")
  local dropped = session:group_count("drop")
  local remaining = session:group_count("todo")
  return string.format(
    "%d of %d kept, %d remaining",
    kept,
    kept + dropped,
    remaining
  )
end

function Review:update_toolbar(session)
  local active = session.composition
  local group = active and active.group
  if self.progress then
    self.progress.text = self:progress_text(session)
  end
  if self.kept then
    self.kept.value = group == "keep"
  end
  if self.dropped then
    self.dropped.value = group == "drop"
  end
end

Review:on("composition_selected", function(self, _composition)
  self:update_toolbar(field.session.shared())
end)

function Review:todo_index(session, todos)
  local active = session.composition
  if not active then
    return 0
  end
  for i, doc in ipairs(todos) do
    if doc.id == active.id then
      return i
    end
  end
  return 0
end

function Review:go_next(session)
  local todos = shared.docs_in_group(session, "todo")
  if #todos == 0 then
    self:update_toolbar(session)
    return
  end
  local ix = self:todo_index(session, todos)
  session.composition = todos[(ix % #todos) + 1]
  self:update_toolbar(session)
end

function Review:go_previous(session)
  local todos = shared.docs_in_group(session, "todo")
  if #todos == 0 then
    self:update_toolbar(session)
    return
  end
  local ix = self:todo_index(session, todos)
  if ix <= 1 then
    session.composition = todos[#todos]
  else
    session.composition = todos[ix - 1]
  end
  self:update_toolbar(session)
end

function Review:set_dropped(session, on)
  local doc = session.composition
  if not doc then
    return false
  end
  if on then
    doc.group = "drop"
  else
    doc.group = "todo"
  end
  self:update_toolbar(session)
  return on
end

function Review:set_keep(session, on)
  local doc = session.composition
  if not doc then
    return false
  end
  if on then
    doc.group = "keep"
  else
    doc.group = "todo"
  end
  self:update_toolbar(session)
  return on
end

function Review:open_todo(session, path)
  if shared.is_session_path(path) then
    return
  end
  local ok, doc = pcall(function()
    return session:open(path)
  end)
  if ok and doc then
    doc.group = "todo"
  end
end

function Review:build_toolbar(session)
  local success = app.theme.semantic.success
  local danger = app.theme.semantic.danger

  self.progress = field.ui.message({
    id = "progress",
    text = "-",
    color = app.theme.semantic.muted_foreground,
  })
  self.kept = field.ui.toggle({
    id = "keep",
    label = "Keep",
    value = false,
    on_color = success,
    on_icon = "circle-check",
    action = function(ctrl, workflow)
      ctrl.value = workflow:set_keep(field.session.shared(), not ctrl.value)
    end,
  })
  self.dropped = field.ui.toggle({
    id = "drop",
    label = "Drop",
    value = false,
    on_color = danger,
    on_icon = "circle-x",
    action = function(ctrl, workflow)
      ctrl.value = workflow:set_dropped(field.session.shared(), not ctrl.value)
    end,
  })

  self:set_toolbar({
    field.ui.button({
      id = "previous",
      label = "Previous",
      icon = "arrow-left",
      action = function(_, workflow)
        workflow:go_previous(field.session.shared())
      end,
    }),
    field.ui.button({
      id = "next",
      label = "Next",
      icon = "arrow-right",
      action = function(_, workflow)
        workflow:go_next(field.session.shared())
      end,
    }),
    self.kept,
    self.dropped,
    field.ui.divider(),
    self.progress,
    field.ui.button({
      id = "finish",
      label = "Finish → Catalog",
      align = "right",
      action = function(_, workflow)
        workflow:finish_to_catalog()
      end,
    }),
  })

  self:update_toolbar(session)
end

function Review:finish_to_catalog()
  local session = field.session.shared()
  self:set_review_playback(false)
  local n = session:group_count("todo")
  if n > 0 then
    field.log.warn("review", string.format("%d document(s) still in todo", n))
  end
  -- Intended handoff:
  --   field.workflow.finish({ next = "catalog" })
  field.workflow.finish()
  app:alert(
    "Review complete",
    "Start Catalog from the Workflow menu to export kept takes."
  )
end

function Review:start(payload)
  field.log.info("review", "starting")
  local session = field.session.shared()
  local scope = payload.scope or "?"
  if scope == "menu" then
    local docs = session.compositions or {}
    for _, doc in ipairs(docs) do
      doc.group = "todo"
    end
    field.log.info("review", string.format("via menu: %d document(s) marked todo", #docs))
    self:build_toolbar(session)
    self:set_review_playback(true)
    app:command("view.show-explorer")
    return
  end
  local incoming = payload.paths or {}
  field.log.info("review", string.format("via drop: %d path(s), scope=%s", #incoming, scope))
  if #incoming == 0 then
    field.log.info("review", "(none)")
  else
    for i, item in ipairs(incoming) do
      field.log.info("review", string.format("%d. %s", i, item))
      for _, path in ipairs(shared.expand_media(item, shared.review_exts())) do
        self:open_todo(session, path)
      end
    end
  end
  self:build_toolbar(session)
  self:set_review_playback(true)
  app:command("view.show-explorer")
end

function Review:suspend(_session)
  return true
end

function Review:resume(session)
  local total = #(session.compositions or {})
  local done = session:group_count("keep")
  local pct = 100
  if total > 0 then
    pct = math.floor((done * 100 / total) + 0.5)
  end
  field.log.info("review", string.format("%d%% (%d/%d)", pct, done, total))
  self:build_toolbar(session)
  self:set_review_playback(true)
  app:command("view.show-explorer")
end

function Review:cancel(_session)
  field.log.info("review", "canceled")
end

function Review:finish(session)
  self:set_review_playback(false)
  local n = session:group_count("todo")
  if n > 0 then
    field.log.warn("review", string.format("%d document(s) still in todo", n))
  end
end

field.workflow.declare(Review)
