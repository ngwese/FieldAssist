-- Built-in drag-drop workflow: log dropped paths (mock).

app:declare_workflow({
  name = "review",
  display_name = "Review",
  description = "Log information about dropped files",
  scopes = { "drag-drop" },
  drop = { row = 2, priority = 1, color = app.theme.semantic.info },
}, function(payload)
  local paths = payload.paths or {}
  local scope = payload.scope or "?"
  app:info("review", string.format("%d path(s), scope=%s", #paths, scope))
  if #paths == 0 then
    app:info("review", "(none)")
    return
  end
  for i, path in ipairs(paths) do
    app:info("review", string.format("%d. %s", i, path))
  end
end)
