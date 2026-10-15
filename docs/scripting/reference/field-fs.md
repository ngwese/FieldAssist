# `field.fs`

<!-- markdownlint-disable MD013 MD060 -->
Filesystem helpers. Paths accept a `string` or (in field-scripting) url
userdata.

## field.fs Properties
(none)

## field.fs Functions
| Function           | Arguments                                | Returns                             | Hosts           | Description                                                                                                                                                  |
| ------------------ | ---------------------------------------- | ----------------------------------- | --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `find_files`       | `dir [, filter]`                         | `{ string, … }`                     | all             | Recursive listing of paths **relative to** `dir`, `/`-separated. `filter` = `nil`, extension list `{ "wav", … }`, or `function(dirname, basename) → truthy`. |
| `mkdir`            | `path [, { recursive = bool }]`          | —                                   | field-scripting | Create directory. Default `recursive = true`.                                                                                                                |
| `remove`           | `path [, { recursive = bool }]`          | —                                   | field-scripting | Remove file or directory. Default `recursive = false`.                                                                                                       |
| `copy`             | `src`, `dest` [, `{ recursive = bool }`] | —                                   | field-scripting | Copy file, or directory when `recursive = true`.                                                                                                             |
| `exists`           | `path`                                   | `boolean`                           | field-scripting | Path exists                                                                                                                                                  |
| `stat`             | `path`                                   | `{ size, is_dir, is_file, mtime? }` | field-scripting | Metadata; `mtime` is unix seconds when available                                                                                                             |
| `available_space`  | `path`                                   | `integer` (bytes)                   | field-scripting | Free bytes usable by non-privileged processes on the volume containing `path`. Walks parents if `path` does not exist yet.                                   |
| `total_space`      | `path`                                   | `integer` (bytes)                   | field-scripting | Total capacity of the volume containing `path` (same ancestor walk as `available_space`).                                                                    |
| `checksum`         | `path [, algo]`                          | `string`                            | field-scripting | Hex digest; only `"blake3"` (default)                                                                                                                        |

---
