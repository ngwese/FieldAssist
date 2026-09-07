-- Built-in drag-drop workflow: add files to the current session.

local function is_session_path(path)
  return string.lower(path):match("%.fasession$") ~= nil
end

local function session_has_path(session, path)
  if not path then
    return false
  end
  for _, doc in ipairs(session.documents) do
    if doc.path == path then
      return true
    end
  end
  return false
end

app:declare_workflow({
  name = "add",
  display_name = "Add",
  description = "Add audio, compositions, or merge a session into the current session",
  scopes = { "drag-drop" },
  drop = { row = 1, priority = 1, color = app.theme.semantic.success },
}, function(payload)
  local paths = payload.paths or {}
  for _, path in ipairs(paths) do
    if is_session_path(path) then
      local incoming = app:load_session(path)
      for _, doc in ipairs(incoming.documents) do
        local doc_path = doc.path
        if doc_path and not session_has_path(app.session, doc_path) then
          app.session:open(doc_path)
        end
      end
      incoming:close()
    else
      app.session:open(path)
    end
  end
end)
