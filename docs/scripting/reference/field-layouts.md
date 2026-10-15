# `field.layouts`

<!-- markdownlint-disable MD013 MD060 -->
Channel layout registry used by detect hooks and composition layout choice.

## field.layouts Properties
(none)

## field.layouts Functions
| Function          | Arguments     | Returns                             | Hosts           | Description                                |
| ----------------- | ------------- | ----------------------------------- | --------------- | ------------------------------------------ |
| `define`          | `spec: table` | —                                   | all             | Sugar for `shared_registry():define(spec)` |
| `shared_registry` | —             | [layout registry](field-layouts.md) | field-scripting | Process layout registry                    |

## field.layouts `define(spec)` keys
| Key           | Required | Type     | Description                                                                  |
| ------------- | -------- | -------- | ---------------------------------------------------------------------------- |
| `name`        | yes      | `string` | Non-empty layout id                                                          |
| `code`        | no       | `string` | Filename-safe token for `${channel_layout}` (default = `name`; empty → `name`) |
| `description` | no       | `string` | Human label (default `""`)                                                   |
| `channels`    | yes      | map      | **0-based** channel index → label string                                     |
| `monitor`     | no       | table    | JSON-compatible monitor table (often `{ chain = "…" }`)                      |

## Returned type: layout registry
### layout registry Properties
(none)

### layout registry Methods
| Method    | Arguments             | Returns         | Description                                 |
| --------- | --------------------- | --------------- | ------------------------------------------- |
| `:define` | `spec: table`         | —               | Register or replace a layout by `name`      |
| `:remove` | layout or name string | —               | Drop a registered layout (no-op if missing) |
| `:items`  | —                     | `{ layout, … }` | Registered layouts in definition order      |

## Returned type: layout
### layout Properties
| Property      | Access | Type           | Description                                              |
| ------------- | ------ | -------------- | -------------------------------------------------------- |
| `name`        | **ro** | `string`       | Layout id                                                |
| `code`        | **ro** | `string`       | Filename-safe token (defaults to `name` when unset)      |
| `description` | **ro** | `string`       | Human label                                              |
| `channels`    | **ro** | map            | **0-based** channel index → label string                 |
| `monitor`     | **ro** | table or `nil` | Monitor table from `define`                              |

### layout Methods
(none)

---
