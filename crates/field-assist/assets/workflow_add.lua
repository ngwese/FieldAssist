-- Built-in drag-drop workflow: add files to the current session.

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

local function session_has_path(session, path)
  if not path then
    return false
  end
  for _, doc in ipairs(session.compositions) do
    if doc.path == path then
      return true
    end
  end
  return false
end

local function add_path(path)
  local session = field.session.focused()
  if is_session_path(path) then
    local incoming = field.session.open(path)
    for _, doc in ipairs(incoming.compositions) do
      local doc_path = doc.path
      if doc_path and not session_has_path(session, doc_path) then
        session:open(doc_path)
      end
    end
    incoming:close()
  else
    session:open(path)
  end
end

local drop_color = (app.theme and app.theme.semantic and app.theme.semantic.success)
  or field.ui.semantic.success

field.workflow.declare({
  name = "add",
  display_name = "Add",
  description = "Add audio, compositions, or merge a session into the current session",
  scopes = { "drag-drop" },
  drop = { row = 1, priority = 1, color = drop_color },
}, function(payload)
  local paths = payload.paths or {}
  for _, item in ipairs(paths) do
    if is_session_path(item) then
      add_path(item)
    else
      for _, path in ipairs(expand_item(item)) do
        add_path(path)
      end
    end
  end
end)
