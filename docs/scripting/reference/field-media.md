# `field.media`

<!-- markdownlint-disable MD013 MD060 -->
Shared media pool (interned media rows used by compositions).

## field.media Properties
(none)

## field.media Functions
| Function      | Arguments | Returns                   | Description                  |
| ------------- | --------- | ------------------------- | ---------------------------- |
| `shared_pool` | —         | [media pool](field-media.md) | Process singleton media pool |
| `open`        | `path` (string or url) | [media](field-media.md) | Probe a file without adding it to the pool (detached media) |
| `transcode`   | `media`, `dest`, `profile` [, `on_progress`] | — | Stream-transcode `media` to `dest` (blocks the host) |
| `begin_transcode` | `media`, `dest`, `profile` | [transcode job](field-media.md) | Same encode on a background thread for UI polling |

`field.media.transcode(media, dest, profile [, on_progress])` writes `dest`
without loading the whole source into memory. `profile` may be a registered
profile name, export-profile userdata, or an options table (`encoder`,
`sample_rate`, `sample_format`, `channels`, …) resolved the same way as
`composition:export`. Sample-rate and PCM format from the profile are applied
(bandlimited SRC when rates differ). Optional `on_progress(done, total)`
receives **source** frame counts. Prefer `begin_transcode` from a sheet so
progress can update between `:defer` turns without freezing the UI.

## Returned type: transcode job
Returned by `field.media.begin_transcode`. Poll from a `:defer` loop.

### transcode job Properties
| Property   | Access | Type      | Description                        |
| ---------- | ------ | --------- | ---------------------------------- |
| `done`     | **ro** | `integer` | Source frames completed            |
| `total`    | **ro** | `integer` | Source frame count (0 until known) |
| `finished` | **ro** | `boolean` | True when the worker has exited    |

### transcode job Methods
| Method      | Arguments | Returns           | Description                                   |
| ----------- | --------- | ----------------- | --------------------------------------------- |
| `:progress` | —         | `done`, `total`   | Same counters as the fields                   |
| `:error`    | —         | `string` or `nil` | Failure message after `finished`              |
| `:join`     | —         | —                 | Block until done; errors if the worker failed |

## Returned type: media pool
### media pool Properties
(none)

### media pool Methods
| Method    | Arguments              | Returns              | Description                            |
| --------- | ---------------------- | -------------------- | -------------------------------------- |
| `:add`    | `path` (string or url) **or** [media](field-media.md) | [media](field-media.md) | Probe+intern a path, or intern detached media (no re-probe); already-pooled media is idempotent |
| `:remove` | media or id string     | `boolean` (headless) | Remove from the pool when unreferenced |
| `:items`  | —                      | `{ media, … }`       | Current pool rows                      |

## Returned type: media
### media Properties
| Property           | Access | Type               | Description                                      |
| ------------------ | ------ | ------------------ | ------------------------------------------------ |
| `id`               | **ro** | `string`           | Media id                                         |
| `url`              | **ro** | url userdata       | Portable location (`:as_path()` for native path) |
| `basename`         | **ro** | `string`           | File basename                                    |
| `sample_rate`      | **ro** | `integer`          | Hz                                               |
| `channels`         | **ro** | `integer`          | Channel count                                    |
| `frames`           | **ro** | `integer`          | Frame count                                      |
| `bit_depth`        | **ro** | `integer` or `nil` | Bits per sample when known                       |
| `size_bytes`       | **ro** | `integer`          | File size                                        |
| `modified`         | **ro** | `string`           | Modification time (host formatting)              |
| `container_format` | **ro** | `string`           | Container                                        |
| `codec`            | **ro** | `string`           | Codec                                            |
| `duration`         | **ro** | `number`           | Seconds                                          |
| `variables`         | **rw** | bindings           | Top-level `source` Bindings (probe + `enrich_media`) |

### media Methods
| Method       | Arguments              | Returns  | Description                                                                 |
| ------------ | ---------------------- | -------- | --------------------------------------------------------------------------- |
| `:bindings`  | `scope?` (`"source"` / `"source.*"`) | bindings | Bindings for that scope; omit `scope` for top-level `source`. Probe tag scopes are read-only; `source` and script-created `source.*` are writable |

---
