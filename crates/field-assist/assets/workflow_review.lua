-- Built-in workflow: review session documents.
--
-- create_workflow builds a prototype table; it is not registered until
-- declare_workflow at the bottom of this file. Defining :suspend or :resume
-- makes it stateful: after :start the host sets app.session.workflow_name = "review".

local Review = app:create_workflow({
  name = "review",
  display_name = "Review",
  description = "Review session documents",
  scopes = { "drag-drop", "menu" },
  drop = { row = 2, priority = 1, color = app.theme.semantic.info },
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

-- Directories are expanded with app:find_files; a file path fails that call
-- and is kept as a single item.
local function expand_item(path)
  local ok, found = pcall(function()
    return app:find_files(path, MEDIA_EXTS)
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

local function progress_text()
  local s = app.session
  local reviewed = s:group_count("reviewed")
  local total = reviewed + s:group_count("todo")
  return string.format("%d of %d files reviewed", reviewed, total)
end

function Review:sync_chrome()
  local active = app.composition
  local on = active ~= nil and active.group == "reviewed"
  self:set_item("progress", { text = progress_text() })
  self:set_item("reviewed", { value = on })
end

local function todo_docs()
  local docs = {}
  for _, doc in ipairs(app.session.compositions or {}) do
    if doc.group == "todo" then
      docs[#docs + 1] = doc
    end
  end
  return docs
end

local function todo_index(todos)
  local active = app.composition
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

function Review:go_next()
  local todos = todo_docs()
  if #todos == 0 then
    self:sync_chrome()
    return
  end
  local ix = todo_index(todos)
  app.composition = todos[(ix % #todos) + 1]
  self:sync_chrome()
end

function Review:go_previous()
  local todos = todo_docs()
  if #todos == 0 then
    self:sync_chrome()
    return
  end
  local ix = todo_index(todos)
  if ix <= 1 then
    app.composition = todos[#todos]
  else
    app.composition = todos[ix - 1]
  end
  self:sync_chrome()
end

function Review:drop_active()
  local doc = app.composition
  if doc then
    doc.group = "drop"
  end
  self:sync_chrome()
end

-- Toggle Reviewed on the active document. Returns the toggle value the host
-- should keep.
function Review:set_reviewed(on)
  local doc = app.composition
  if not doc then
    return false
  end
  if on then
    doc.group = "reviewed"
  else
    doc.group = "todo"
  end
  self:sync_chrome()
  return on
end

function Review:show_toolbar()
  local off = app.theme.semantic.muted_foreground
  local on = app.theme.semantic.success
  self:set_toolbar({
    { command = "previous", label = "Previous" },
    { command = "next", label = "Next" },
    { command = "drop", label = "Drop" },
    {
      id = "reviewed",
      kind = "toggle",
      label = "Reviewed",
      value = false,
      off_color = off,
      on_color = on,
      on_change = function(current)
        return self:set_reviewed(not current)
      end,
    },
    { kind = "divider" },
    { id = "progress", kind = "message", text = progress_text() },
    {
      id = "output",
      kind = "path",
      label = "Output",
      value = self.output or "",
      browse = "directory",
      align = "right",
      on_path = function(paths)
        local path = paths and paths[1] or nil
        if not path or path == "" then
          return self.output or ""
        end
        self.output = path
        return path
      end,
    },
    { command = "finish", label = "Finish", align = "right" },
  })
  self:sync_chrome()
end

local function open_todo(path)
  if is_session_path(path) then
    return
  end
  local ok, doc = pcall(function()
    return app.session:open(path)
  end)
  if ok and doc then
    doc.group = "todo"
  end
end

function Review:restore_output(session)
  local s = session or app.session
  local props = s.properties or {}
  self.output = self.output or props.output or ""
end

-- Host calls :start from the Workflow menu (`scope == "menu"`) or when files
-- are dropped on the Review overlay cell. Dropping Review again while it is
-- already running creates a new instance and calls :start again. Folders are
-- expanded to readable audio/.facomp files. After this returns, the host binds
-- the session.
function Review:start(payload)
  app:info("review", "starting")
  self:restore_output(app.session)
  local scope = payload.scope or "?"
  if scope == "menu" then
    local s = app.session
    local docs = s.compositions or {}
    for _, doc in ipairs(docs) do
      doc.group = "todo"
    end
    app:info("review", string.format("via menu: %d document(s) marked todo", #docs))
    self:show_toolbar()
    self:set_review_playback(true)
    app:command("view.show-explorer")
    return
  end
  local incoming = payload.paths or {}
  app:info("review", string.format("via drop: %d path(s), scope=%s", #incoming, scope))
  if #incoming == 0 then
    app:info("review", "(none)")
  else
    for i, item in ipairs(incoming) do
      app:info("review", string.format("%d. %s", i, item))
      for _, path in ipairs(expand_item(item)) do
        open_todo(path)
      end
    end
  end
  self:show_toolbar()
  self:set_review_playback(true)
  app:command("view.show-explorer")
end

-- Host calls this when a toolbar button is clicked. `command` is the string
-- from set_toolbar (previous / next / drop / finish). Finish asks the host to
-- end the run, which then calls :finish below.
Review:on("command", function(self, command)
  if command == "previous" then
    self:go_previous()
  elseif command == "next" then
    self:go_next()
  elseif command == "drop" then
    self:drop_active()
  elseif command == "finish" then
    app:finish_workflow()
  end
end)

-- Host calls :suspend before Save Session / Save Session As, and during quit
-- or open-session after compositions are clean (before the unsaved-session
-- prompt). Return false to abort that save or quit.
function Review:suspend(session)
  local s = session or app.session
  local props = s.properties or {}
  props.output = self.output or props.output or ""
  s.properties = props
  return true
end

-- Host calls :resume after a .fasession is installed as the UI session when
-- s.workflow_name is "review". Restore the toolbar, output path, and playback chrome.
function Review:resume(session)
  local s = session or app.session
  self:restore_output(s)
  local total = #(s.compositions or {})
  local todo = s:group_count("todo")
  local done = s:group_count("reviewed")
  local pct = 100
  if total > 0 then
    pct = math.floor((done * 100 / total) + 0.5)
  end
  app:info("review", string.format("%d%% (%d/%d)", pct, done, total))
  self:show_toolbar()
  self:set_review_playback(true)
  app:command("view.show-explorer")
end

-- Host calls :cancel when the running session is discarded (replaced by
-- another .fasession without keeping this run) or when a script calls
-- app:cancel_workflow().
function Review:cancel(_session)
  app:info("review", "canceled")
end

-- Host calls :finish after app:finish_workflow() (here, the Finish button).
-- Then the host clears s.workflow_name and hides the toolbar.
function Review:finish(session)
  self:set_review_playback(false)
  local s = session or app.session
  local n = s:group_count("todo")
  if n > 0 then
    app:warn("review", string.format("%d document(s) still in todo", n))
  end
end

-- Register last so later user workflow_review.lua can override this name.
app:declare_workflow(Review)
