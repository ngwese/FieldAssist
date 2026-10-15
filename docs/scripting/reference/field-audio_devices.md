# `field.audio_devices`

<!-- markdownlint-disable MD013 MD060 -->
Enumerate playback output devices (via `field-audio-playback`).

## field.audio_devices Properties
(none)

## field.audio_devices Functions
| Function | Arguments | Returns         | Description                         |
| -------- | --------- | --------------- | ----------------------------------- |
| `list`   | —         | `{ device, … }` | Output devices (may be empty in CI) |

Each device table has:

| Property        | Type               | Description                  |
| --------------- | ------------------ | ---------------------------- |
| `index`         | `integer`          | Host device list index       |
| `name`          | `string`           | Display name                 |
| `is_default`    | `boolean`          | Host default output          |
| `sample_rate`   | `integer` or `nil` | Default config Hz            |
| `channels`      | `integer` or `nil` | Default config channel count |
| `sample_format` | `string` or `nil`  | e.g. `F32`, `I16`            |

**Index:** field-scripting uses the device’s native `index`. FieldAssist’s
`field.audio_devices.list` currently returns `{ index, name }` only
(1-based `index`).

---
