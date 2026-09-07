-- Built-in workflow: review session documents (stateful mock).
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

-- Toolbar clicks send workflow-local command strings (not app:command ids).
local function show_toolbar()
  Review:set_toolbar({
    { command = "next", label = "Next" },
    { command = "skip", label = "Skip" },
    { command = "finish", label = "Finish" },
  })
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
end

-- Host calls this when a toolbar button is clicked. `command` is the string
-- from set_toolbar (next / skip / finish). Finish asks the host to end the
-- run, which then calls :finish below.
Review:on("command", function(command)
  app:info("review", string.format("command %s", tostring(command)))
  if command == "finish" then
    app:finish_workflow()
  end
end)

-- Host calls :suspend before Save Session / Save Session As, and during quit
-- or open-session after compositions are clean (before the unsaved-session
-- prompt). Return false to abort that save or quit.
function Review:suspend(_session)
  return true
end

-- Host calls :resume after a .fasession is installed as the UI session when
-- s.workflow is "review". Cache session document counts on this prototype,
-- log completion, and restore the toolbar.
function Review:resume(session)
  local s = session or app.session
  self.total = #(s.documents or {})
  self.todo = s:group_count("todo")
  local done = self.total - self.todo
  local pct = 100
  if self.total > 0 then
    pct = math.floor((done * 100 / self.total) + 0.5)
  end
  app:info("review", string.format("%d%% (%d/%d)", pct, done, self.total))
  show_toolbar()
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
  local s = session or app.session
  local n = s:group_count("todo")
  if n > 0 then
    app:warn("review", string.format("%d document(s) still in todo", n))
  end
end

-- Register last so later user workflow_review.lua can override this name.
app:declare_workflow(Review)
