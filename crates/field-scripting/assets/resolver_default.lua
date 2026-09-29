-- Default variable resolver: last-wins across the bindings list passed to :init.
-- Loaded before init.lua so `"default"` is always registered.

local Default = field.variables.create_resolver({ name = "default" })

function Default:init(bindings)
  self._bindings = bindings or {}
end

function Default:names()
  local seen = {}
  local out = {}
  for _, b in ipairs(self._bindings) do
    for _, name in ipairs(b:names()) do
      if not seen[name] then
        seen[name] = true
        out[#out + 1] = name
      end
    end
  end
  return out
end

function Default:resolve(scope, name)
  -- Virtual scope: process environment (not a compose-layer Bindings).
  if scope == "env" then
    local v = os.getenv(name)
    if v ~= nil then
      return v, "env"
    end
    return nil, nil
  end
  if scope ~= nil then
    for i = #self._bindings, 1, -1 do
      local b = self._bindings[i]
      if b:scope() == scope then
        local v = b.values[name]
        if v ~= nil then
          return v, scope
        end
      end
    end
    return nil, nil
  end
  for i = #self._bindings, 1, -1 do
    local b = self._bindings[i]
    local v = b.values[name]
    if v ~= nil then
      return v, b:scope()
    end
  end
  return nil, nil
end

field.variables.declare_resolver(Default)
field.variables.set_resolver("default")
