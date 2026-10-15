# `field.exports`

<!-- markdownlint-disable MD013 MD060 -->
Export profile registry for scripted one-shot encodes (`composition:export`).
Profiles express the same options as the File → Export sheet.

## field.exports Properties
(none)

## field.exports Functions
| Function          | Arguments     | Returns                             | Hosts           | Description                                |
| ----------------- | ------------- | ----------------------------------- | --------------- | ------------------------------------------ |
| `define`          | `spec: table` | —                                   | all             | Sugar for `shared_registry():define(spec)` |
| `shared_registry` | —             | [export registry](field-exports.md) | field-scripting | Process export profile registry            |

## field.exports `define(spec)` keys
| Key             | Required | Type                      | Description                                                                            |
| --------------- | -------- | ------------------------- | -------------------------------------------------------------------------------------- |
| `name`          | yes      | `string`                  | Non-empty profile id                                                                   |
| `description`   | no       | `string`                  | Human label (default `""`)                                                             |
| `encoder`       | no       | `string`                  | Encoder id (`wav`, `flac`, `ogg`; default `wav` at export)                             |
| `extension`     | no       | `string`                  | Output file extension without a leading dot (default: same as `encoder`)               |
| `sample_format` | no       | `string`                  | PCM label (`S16`, `S24`, `F32`, …); snapped when the encoder stores PCM                |
| `sample_rate`   | no       | `integer`                 | Output Hz (default: composition rate)                                                  |
| `channels`      | no       | `"all"` or `{ indices… }` | **0-based** channel indices (default all)                                              |
| `directory`     | no       | string or url             | Output directory (`${…}` interpolated)                                                 |
| `filename`      | no       | `string`                  | Output filename (`${…}` interpolated)                                                  |
| `path`          | no       | string or url             | Full destination; wins over `directory` + `filename` (`${…}` interpolated)             |
| `variables`     | no       | table                     | Export-scoped variables (same shapes as `session.variables`)                           |
| `metadata`      | no       | `{ [string] = string }`   | Canonical tag templates (`title`, `artist`, …) interpolated then written into the file |

## `composition:export(arg)`
`arg` may be:

- a profile **name** string or profile **userdata**
- an options table: ad-hoc encode settings, and/or `{ profile = name, …overrides }`
- `{ name = "profile", path = "…", … }` when `name` matches a registered profile

Destination requires `path`, or `directory` (with optional `filename`). Returns
`true` on success; raises a Lua error on failure. Encode matches File → Export
(no monitor DSP). Processing chains and background batch jobs are future work.

## Setting resolution
Export settings are resolved in layers. A later layer supplies a value **only
when that field is explicitly set**; omitted fields keep the previous layer.

1. **Source defaults** from the open composition / primary media:

   - `sample_rate` ← composition sample rate
   - `sample_format` ← primary media bit depth (else preference `S24`)
   - `channels` ← all composition channels
   - `encoder` ← product default `"wav"`
   - destination ← unset (caller must supply `path` or `directory`)

2. **Profile** (`field.exports.define` / named profile): each set field
   overrides the source for that field only
3. **Call-time overrides** (inline table keys, or Export-sheet UI edits after
   picking a profile): same sparse overlay as a profile

After merging, the encoder's capabilities snap `sample_format` and validate
rate / channel count. File → Export applies the same source ← profile merge
when a Profile is chosen in the sheet; opening the sheet with **Custom** uses
source ← session `export.*` prefs instead.

The Export sheet also maintains an **export** variable layer: the selected
profile’s `variables` table, plus live Format/channel upserts so path
templates can reference encode settings:

| Leaf | Meaning |
| --- | --- |
| `encoder` | Encoder id |
| `sample_format` | PCM label, or `""` when the encoder does not store PCM |
| `sample_rate` | Output Hz as a string |
| `channels` | Selected channel count as a string |
| `extension` | Encoder file extension |

Export-site compose (source → user → session → composition → export) feeds the
sheet’s Variables disclosure, the soft **Resolved** path preview, and (on
Export) strict path interpolate plus tag map build — matching
`composition:export` / `build_job`.

## Returned type: export registry
### export registry Properties
(none)

### export registry Methods
| Method    | Arguments              | Returns          | Description                                  |
| --------- | ---------------------- | ---------------- | -------------------------------------------- |
| `:define` | `spec: table`          | —                | Register or replace a profile by `name`      |
| `:remove` | profile or name string | —                | Drop a registered profile (no-op if missing) |
| `:find`   | `name: string`         | profile or `nil` | Look up a registered profile by name         |
| `:names`  | —                      | `{ string, … }`  | Registered profile names in definition order |
| `:items`  | —                      | `{ profile, … }` | Registered profiles in definition order      |

## Returned type: export profile
### export profile Properties
| Property        | Access | Type                      | Description       |
| --------------- | ------ | ------------------------- | ----------------- |
| `name`          | **ro** | `string`                  | Profile id        |
| `description`   | **ro** | `string`                  | Human label       |
| `encoder`       | **ro** | `string` or `nil`         | Encoder id        |
| `extension`     | **ro** | `string`                  | File extension (defaults to `encoder`, else `"wav"`) |
| `sample_format` | **ro** | `string` or `nil`         | PCM label         |
| `sample_rate`   | **ro** | `integer` or `nil`        | Hz                |
| `channels`      | **ro** | `"all"` or `{ indices… }` | Channel selection |
| `directory`     | **ro** | `string` or `nil`         | Directory         |
| `filename`      | **ro** | `string` or `nil`         | Filename          |
| `path`          | **ro** | `string` or `nil`         | Full path         |

### export profile Methods
(none)

---

---
