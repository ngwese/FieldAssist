-- Built-in workflow: review session documents.
--
-- field.workflow.create builds a prototype table; it is not registered until
-- field.workflow.declare at the bottom of this file. Defining :suspend or
-- :resume makes it stateful: after :start the host sets
-- field.session.focused().workflow_name = "review".

local drop_color = (app.theme and app.theme.semantic and app.theme.semantic.info)
  or field.ui.semantic.info

local Review = field.workflow.create({
  name = "review",
  display_name = "Review",
  description = "Review session documents",
  scopes = { "drag-drop", "menu" },
  drop = { row = 2, priority = 1, color = drop_color },
})

-- Readable audio plus composition projects. find_files matches these
-- case-insensitively, with or without a leading dot.
local MEDIA_EXTS = {
  "wav", "wave", "aif", "aiff", "flac", "ogg", "oga",
  "mp3", "mp2", "m4a", "aac", "caf", "w64", "facomp",
}

local function join_path(dir, rel)
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

local function is_session_path(path)
  return string.lower(path):match("%.fasession$") ~= nil
end

-- Directories are expanded with field.fs.find_files; a file path fails that
-- call and is kept as a single item.
local function expand_item(path)
  local ok, found = pcall(function()
    return field.fs.find_files(path, MEDIA_EXTS)
  end)
  if not ok then
    return { path }
  end
  local paths = {}
  for _, rel in ipairs(found) do
    paths[#paths + 1] = join_path(path, rel)
  end
  return paths
end

local function ensure_flag(id, want)
  local current = (id == "transport.loop") and app.looping or app.preview
  if current ~= want then
    app:command(id)
  end
end

function Review:set_review_playback(on)
  ensure_flag("transport.loop", on)
  ensure_flag("transport.preview", on)
end

local function progress_text(session)
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
    self.progress.text = progress_text(session)
  end
  if self.kept then
    self.kept.value = group == "keep"
  end
  if self.dropped then
    self.dropped.value = group == "drop"
  end
end

-- Dropped when the run ends. A click in the compositions pane focuses that
-- document and should refresh the Keep toggle.
Review:on("composition_selected", function(self, _composition)
  self:update_toolbar(field.session.focused())
end)

local function todo_docs(session)
  local docs = {}
  for _, doc in ipairs(session.compositions or {}) do
    if doc.group == "todo" then
      docs[#docs + 1] = doc
    end
  end
  return docs
end

local function todo_index(session, todos)
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
  local todos = todo_docs(session)
  if #todos == 0 then
    self:update_toolbar(session)
    return
  end
  local ix = todo_index(session, todos)
  session.composition = todos[(ix % #todos) + 1]
  self:update_toolbar(session)
end

function Review:go_previous(session)
  local todos = todo_docs(session)
  if #todos == 0 then
    self:update_toolbar(session)
    return
  end
  local ix = todo_index(session, todos)
  if ix <= 1 then
    session.composition = todos[#todos]
  else
    session.composition = todos[ix - 1]
  end
  self:update_toolbar(session)
end

-- Toggle Drop on the active document. Returns the toggle value the host
-- should keep.
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

-- Toggle Keep on the active document. Returns the toggle value the host
-- should keep.
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

function Review:build_toolbar(session)
  local theme = app.theme or field.ui
  local success = theme.semantic.success
  local danger = theme.semantic.danger

  -- Set these on the workflow instance so that they can be updated by other methods.
  self.progress = field.ui.message({
    id = "progress",
    text = "-",
    color = theme.semantic.muted_foreground,
  })
  self.kept = field.ui.toggle({
    id = "keep",
    label = "Keep",
    value = false,
    on_color = success,
    on_icon = "circle-check",
    action = function(ctrl, workflow)
      ctrl.value = workflow:set_keep(field.session.focused(), not ctrl.value)
    end,
  })
  self.dropped = field.ui.toggle({
    id = "drop",
    label = "Drop",
    value = false,
    on_color = danger,
    on_icon = "circle-x",
    action = function(ctrl, workflow)
      ctrl.value = workflow:set_dropped(field.session.focused(), not ctrl.value)
    end,
  })

  self:set_toolbar({
    field.ui.button({
      id = "previous",
      label = "Previous",
      icon = "arrow-left",
      action = function(_, workflow)
        workflow:go_previous(field.session.focused())
      end,
    }),
    field.ui.button({
      id = "next",
      label = "Next",
      icon = "arrow-right",
      action = function(_, workflow)
        workflow:go_next(field.session.focused())
      end,
    }),
    self.kept,
    self.dropped,
    field.ui.divider(),
    self.progress,
    field.ui.path_entry({
      id = "output",
      label = "Output",
      value = self.output or "",
      browse = "directory",
      align = "right",
      action = function(ctrl, workflow)
        workflow.output = ctrl.value or ""
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

  -- Update the toolbar to reflect the current state.
  self:update_toolbar(session)
end

local function open_todo(session, path)
  if is_session_path(path) then
    return
  end
  local ok, doc = pcall(function()
    return session:open(path)
  end)
  if ok and doc then
    doc.group = "todo"
  end
end

function Review:restore_output(session)
  local props = session.properties or {}
  self.output = self.output or props.output or ""
end

-- Host calls :start from the Workflow menu (`scope == "menu"`) or when files
-- are dropped on the Review overlay cell. Dropping Review again while it is
-- already running creates a new instance and calls :start again. Folders are
-- expanded to readable audio/.facomp files. After this returns, the host binds
-- the session.
function Review:start(payload)
  field.log.info("review", "starting")
  local session = field.session.focused()
  self:restore_output(session)
  local scope = payload.scope or "?"
  if scope == "menu" then
    local docs = session.compositions or {}
    for _, doc in ipairs(docs) do
      doc.group = "todo"
    end
    field.log.info("review", string.format("via menu: %d document(s) marked todo", #docs))
    self:build_toolbar(session)
    self:set_review_playback(true)
    if app.name == "field-assist" and app.command then
      app:command("view.show-explorer")
    end
    return
  end
  local incoming = payload.paths or {}
  field.log.info("review", string.format("via drop: %d path(s), scope=%s", #incoming, scope))
  if #incoming == 0 then
    field.log.info("review", "(none)")
  else
    for i, item in ipairs(incoming) do
      field.log.info("review", string.format("%d. %s", i, item))
      for _, path in ipairs(expand_item(item)) do
        open_todo(session, path)
      end
    end
  end
  self:build_toolbar(session)
  self:set_review_playback(true)
  if app.name == "field-assist" and app.command then
    app:command("view.show-explorer")
  end
end

-- Host calls :suspend before Save Session / Save Session As, and during quit
-- or open-session after compositions are clean (before the unsaved-session
-- prompt). Return false to abort that save or quit.
function Review:suspend(session)
  local props = session.properties or {}
  props.output = self.output or props.output or ""
  session.properties = props
  return true
end

-- Host calls :resume after a .fasession is installed as the UI session when
-- session.workflow_name is "review". Restore the toolbar, output path, and
-- playback chrome.
function Review:resume(session)
  self:restore_output(session)
  local total = #(session.compositions or {})
  local todo = session:group_count("todo")
  local done = session:group_count("keep")
  local pct = 100
  if total > 0 then
    pct = math.floor((done * 100 / total) + 0.5)
  end
  field.log.info("review", string.format("%d%% (%d/%d)", pct, done, total))
  self:build_toolbar(session)
  self:set_review_playback(true)
  if app.name == "field-assist" and app.command then
    app:command("view.show-explorer")
  end
end

-- Host calls :cancel when the running session is discarded (replaced by
-- another .fasession without keeping this run) or when a script calls
-- field.workflow.cancel().
function Review:cancel(_session)
  field.log.info("review", "canceled")
end

-- Host calls :finish after field.workflow.finish() (here, the Finish button).
-- Then the host clears session.workflow_name and hides the toolbar.
function Review:finish(session)
  self:set_review_playback(false)
  local n = session:group_count("todo")
  if n > 0 then
    field.log.warn("review", string.format("%d document(s) still in todo", n))
  end
end

-- Register last so later user workflow_review.lua can override this name.
field.workflow.declare(Review)
