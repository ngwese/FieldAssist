-- Built-in workflow: review session documents.
--
-- create_workflow builds a prototype table; it is not registered until
-- declare_workflow at the bottom of this file. Defining :suspend or :resume
-- makes it stateful: after :start the host sets app.session.workflow = "review".

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

local function set_review_playback(on)
  ensure_flag("transport.loop", on)
  ensure_flag("transport.preview", on)
end

local function progress_text()
  local s = app.session
  local reviewed = s:group_count("reviewed")
  local total = reviewed + s:group_count("todo")
  return string.format("%d of %d files reviewed", reviewed, total)
end

local function sync_chrome()
  local active = app.active
  local on = active ~= nil and active.group == "reviewed"
  Review:set_item("progress", { text = progress_text() })
  Review:set_item("reviewed", { value = on })
end

local function show_toolbar()
  local off = app.theme.semantic.muted_foreground
  local on = app.theme.semantic.success
  Review:set_toolbar({
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
        local doc = app.active
        if not doc then
          return current
        end
        local next_on = not current
        if next_on then
          doc.group = "reviewed"
        else
          doc.group = "todo"
        end
        sync_chrome()
        return next_on
      end,
    },
    { kind = "divider" },
    { id = "progress", kind = "message", text = progress_text() },
    {
      id = "output",
      kind = "path",
      label = "Output",
      value = Review.output or "",
      browse = "directory",
      align = "right",
      on_path = function(paths)
        local path = paths and paths[1] or nil
        if not path or path == "" then
          return Review.output or ""
        end
        Review.output = path
        return path
      end,
    },
    { command = "finish", label = "Finish", align = "right" },
  })
  sync_chrome()
end

local function todo_docs()
  local docs = {}
  for _, doc in ipairs(app.session.documents or {}) do
    if doc.group == "todo" then
      docs[#docs + 1] = doc
    end
  end
  return docs
end

local function todo_index(todos)
  local active = app.active
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

local function go_next()
  local todos = todo_docs()
  if #todos == 0 then
    sync_chrome()
    return
  end
  local ix = todo_index(todos)
  app.active = todos[(ix % #todos) + 1]
  sync_chrome()
end

local function go_previous()
  local todos = todo_docs()
  if #todos == 0 then
    sync_chrome()
    return
  end
  local ix = todo_index(todos)
  if ix <= 1 then
    app.active = todos[#todos]
  else
    app.active = todos[ix - 1]
  end
  sync_chrome()
end

local function drop_active()
  local doc = app.active
  if doc then
    doc.group = "drop"
  end
  sync_chrome()
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

-- Host calls :start from the Workflow menu (`scope == "menu"`) or when files
-- are dropped on the Review overlay cell. Dropping Review again while it is
-- already running calls :start a second time. Folders are expanded to readable
-- audio/.facomp files. After this returns, the host binds the session.
function Review:start(payload)
  local scope = payload.scope or "?"
  if scope == "menu" then
    local s = app.session
    local docs = s.documents or {}
    for _, doc in ipairs(docs) do
      doc.group = "todo"
    end
    app:info("review", string.format("menu: %d document(s) marked todo", #docs))
    show_toolbar()
    set_review_playback(true)
    app:command("view.show-explorer")
    return
  end
  local incoming = payload.paths or {}
  app:info("review", string.format("%d path(s), scope=%s", #incoming, scope))
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
  show_toolbar()
  set_review_playback(true)
  app:command("view.show-explorer")
end

-- Host calls this when a toolbar button is clicked. `command` is the string
-- from set_toolbar (previous / next / drop / finish). Finish asks the host to
-- end the run, which then calls :finish below.
Review:on("command", function(command)
  if command == "previous" then
    go_previous()
  elseif command == "next" then
    go_next()
  elseif command == "drop" then
    drop_active()
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
  props.output = Review.output or props.output or ""
  s.properties = props
  return true
end

-- Host calls :resume after a .fasession is installed as the UI session when
-- s.workflow is "review". Restore the toolbar, output path, and playback chrome.
function Review:resume(session)
  local s = session or app.session
  local props = s.properties or {}
  Review.output = props.output or Review.output or ""
  local total = #(s.documents or {})
  local todo = s:group_count("todo")
  local done = s:group_count("reviewed")
  local pct = 100
  if total > 0 then
    pct = math.floor((done * 100 / total) + 0.5)
  end
  app:info("review", string.format("%d%% (%d/%d)", pct, done, total))
  show_toolbar()
  set_review_playback(true)
  app:command("view.show-explorer")
end

-- Host calls :cancel when the running session is discarded (replaced by
-- another .fasession without keeping this run) or when a script calls
-- app:cancel_workflow().
function Review:cancel(_session)
  app:info("review", "canceled")
end

-- Host calls :finish after app:finish_workflow() (here, the Finish button).
-- Then the host clears s.workflow and hides the toolbar.
function Review:finish(session)
  set_review_playback(false)
  local s = session or app.session
  local n = s:group_count("todo")
  if n > 0 then
    app:warn("review", string.format("%d document(s) still in todo", n))
  end
end

-- Register last so later user workflow_review.lua can override this name.
app:declare_workflow(Review)
