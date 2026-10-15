# `field.log`

<!-- markdownlint-disable MD013 MD060 -->
Structured logging into the host log sink (Script panel / stderr / Messages).

## field.log Properties
(none)

## field.log Functions
| Function | Arguments          | Returns | Description        |
| -------- | ------------------ | ------- | ------------------ |
| `info`   | `topic`, `message` | —       | Informational line |
| `warn`   | `topic`, `message` | —       | Warning line       |
| `error`  | `topic`, `message` | —       | Error line         |

On FieldAssist, `topic` / `message` may be any Lua values (stringified). On
field-batch they are strings.

---
