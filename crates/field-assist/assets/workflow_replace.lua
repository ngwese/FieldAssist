-- Built-in drag-drop workflow: replace the session or the active document.

local function is_session_path(path)
  return string.lower(path):match("%.fasession$") ~= nil
end

local drop_color = (app.theme and app.theme.semantic and app.theme.semantic.warning)
  or field.ui.semantic.warning

field.workflow.declare({
  name = "replace",
  display_name = "Replace",
  description = "Replace the active session or the active document",
  scopes = { "drag-drop" },
  drop = { row = 1, priority = 1, color = drop_color },
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
  local session = field.session.shared()
  if is_session_path(path) then
    session:open(path)
    return
  end
  local active = session.composition
  if active then
    active:replace(path)
  else
    session:open(path)
  end
end)
