# `field.scripting`

<!-- markdownlint-disable MD013 MD060 -->
Host controls for Lua `require` search paths and native C modules. Paths are
locked down so workflows stay self-contained by default.

## field.scripting Properties
(none)

## Default `package.path` / `package.cpath`
| Kind | Templates                                                          |
| ---- | ------------------------------------------------------------------ |
| Lua  | `{config}/?.lua`, `{config}/?/init.lua`, `./?.lua`, `./?/init.lua` |
| C    | `{config}/?.so` (`.dll` on Windows), `./?.so` / `./?.dll`          |

`{config}` is the host config directory when known. System-wide bundled Lua
locations (`/usr/local/...`, Windows `!\\...`) and foreign `LUA_PATH*` /
`LUA_CPATH*` roots are **not** on the path until opted in. C loaders remain
stubbed until opted in even if a `.so` sits on `package.cpath`.

Prefer `[field.include](field-scripting.md)` for FieldAssist/field-batch Lua that
should resolve relative to the caller or config dir.

## field.scripting Functions
| Function                      | Arguments | Returns | Description                                                    |
| ----------------------------- | --------- | ------- | -------------------------------------------------------------- |
| `enable_system_package_paths` | —         | —       | Append the saved system-wide path/cpath templates (idempotent) |
| `enable_native_modules`       | —         | —       | Restore `package.loadlib` and C searchers (idempotent)         |

Either call is independent. Typical place is the top of user `init.lua`:

```lua
field.scripting.enable_system_package_paths()
field.scripting.enable_native_modules()
```

For a worked example that builds a Lua 5.4–compatible `lsqlite3.so` for
field-batch and indexes audio paths into SQLite, see
[examples/field-batch/lsqlite-module/](../../examples/field-batch/lsqlite-module/).

---
