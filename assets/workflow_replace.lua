-- Built-in drag-drop workflow: replace the session or the active document.

local function is_session_path(path)
  return string.lower(path):match("%.fasession$") ~= nil
end

app:declare_workflow({
  name = "replace",
  display_name = "Replace",
  description = "Replace the active session or the active document",
  scopes = { "drag-drop" },
  drop = { row = 1, priority = 1, color = app.theme.semantic.warning },
}, function(payload)
  local paths = payload.paths or {}
  if #paths ~= 1 then
    app:alert(
      "Cannot replace",
      "Drop a single file to replace the active session or document."
    )
    return
  end
  local path = paths[1]
  if is_session_path(path) then
    app.session:open(path)
    return
  end
  local active = app.active
  if active then
    active:replace(path)
  else
    app.session:open(path)
  end
end)
