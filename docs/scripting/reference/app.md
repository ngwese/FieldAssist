# `app` (host-only)

<!-- markdownlint-disable MD013 MD060 -->
Process facade unique to the enclosing binary. Model constructors and shared
APIs live under `field.*`, not here.

## app Properties
| Property         | Access | Hosts       | Type              | Description                                                             |
| ---------------- | ------ | ----------- | ----------------- | ----------------------------------------------------------------------- |
| `name`           | **ro** | all         | `string`          | Stable process id (`"field-assist"` / `"field-batch"` / `"field-play"`) |
| `args`           | **ro** | field-batch | `{ string, … }`   | Script argv after the file / `--`                                       |
| `session`        | **ro** | all         | session           | Sugar for `field.session.focused()`                                     |
| `composition`    | **rw** | all         | composition or `nil` | Sugar for `app.session.composition`                                  |
| `workflow`       | **ro** | all         | `table` or `nil`  | Active workflow instance                                                |
| `theme`          | **ro** | FieldAssist | theme userdata    | Live GPUI theme (see below)                                             |
| `themes`         | **ro** | FieldAssist | `{ string, … }`   | Registered theme names                                                  |
| `looping`        | **ro** | FieldAssist | `boolean`         | Transport loop flag                                                     |
| `preview`        | **ro** | FieldAssist | `boolean`         | Preview playback flag                                                   |
| `explorer`       | **ro** | FieldAssist | `boolean`         | Explorer dock open                                                      |
| `output_device`  | **rw** | FieldAssist | `string` or `nil` | Preferred output device (substring match)                               |
| `output_devices` | **ro** | FieldAssist | `{ string, … }`   | Known output device names                                               |

## app Functions / methods
| Function            | Arguments                        | Returns   | Hosts       | Description                                                                                                             |
| ------------------- | -------------------------------- | --------- | ----------- | ----------------------------------------------------------------------------------------------------------------------- |
| `app:alert`         | `subject`, `body`                | —         | all         | Host alert (dialog / stderr). Args are stringified.                                                                     |
| `app:confirm`       | `subject`, `body` [, `options`]  | `boolean` | all         | Confirm gate. Headless / batch / tests: pops `ScriptHost::push_confirm_response` or returns `true`. **FieldAssist** opens a native alert dialog when no queued response is set; the sync return is then `false`, and workflows should put destructive work in `options.on_confirm`. Options (FieldAssist): `ok` / `cancel` labels, `danger` (danger OK button), `on_confirm` / `on_cancel` callbacks. |
| `app:command`       | `id: string`                     | —         | FieldAssist | Invoke a UI command (e.g. `"view.show-explorer"`)                                                                       |
| `app:load_settings` | —                                | —         | FieldAssist | Manual reload: read `settings.json` (or defaults), update the Global store, re-apply theme / docks / waveform / selection / output device / scripting policy |

field-scripting also keeps temporary migration shims
`app:info` / `app:warn` / `app:error` and
`app.finish_workflow` / `app.cancel_workflow` / `app.run_workflow`; prefer
`field.log` and `field.workflow`.

## `app.theme` (FieldAssist)
| Property   | Access | Type                 | Description                                 |
| ---------- | ------ | -------------------- | ------------------------------------------- |
| `name`     | **rw** | `string`             | Theme registry name                         |
| `mode`     | **rw** | `"light"` \| `"dark"` | Appearance mode                             |
| `named`    | **ro** | palette userdata     | Same keys as [field.ui.named](app.md)    |
| `semantic` | **ro** | palette userdata     | Same keys as [field.ui.semantic](app.md) |

```lua
-- Manual reload (startup already applied settings from Rust):
if app.name == "field-assist" and app.load_settings then
  app:load_settings()
end

if app.name == "field-assist" and app.command then
  app:command("view.show-explorer")
end
```

---
