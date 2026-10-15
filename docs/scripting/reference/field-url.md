# `field.url`

<!-- markdownlint-disable MD013 MD060 -->
Location userdata over `field_core::Location` (relative, `file://`, or
`memory://` URLs). Bound on all hosts that use `field-scripting` (including
FieldAssist).

## field.url Properties
(none on the module table)

## field.url Functions
| Function         | Arguments       | Returns      | Description                                              |
| ---------------- | --------------- | ------------ | -------------------------------------------------------- |
| `parse`          | `input: string` | url userdata | Parse a path, `file://`, relative, or `memory://` string |
| `from_path`      | `path: string`  | url userdata | Encode a filesystem path                                 |
| `from_file_path` | `path: string`  | url userdata | Alias of `from_path`                                     |

## Returned type: url userdata
### url Properties
| Property     | Access | Type              | Description                            |
| ------------ | ------ | ----------------- | -------------------------------------- |
| `scheme`     | **ro** | `string`          | e.g. `file`, `memory`, or URL scheme   |
| `host`       | **ro** | `string` or `nil` | Host component when present            |
| `path`       | **ro** | `string`          | Path component of the URL              |
| `query`      | **ro** | `string` or `nil` | Query string                           |
| `native`     | **ro** | `string`          | Native filesystem path when applicable |
| `normalized` | **ro** | `string`          | Canonical serialized form              |
| `parent`     | **ro** | url userdata      | Parent directory (filesystem)          |
| `basename`   | **ro** | `string`          | Final path segment                     |
| `stem`       | **ro** | `string`          | Basename without extension             |
| `extension`  | **ro** | `string`          | Extension without leading `.`          |

### url Methods
| Method            | Arguments                 | Returns      | Description                              |
| ----------------- | ------------------------- | ------------ | ---------------------------------------- |
| `:as_path()`      | —                         | `string`     | Platform-native filesystem path          |
| `:join(...)`      | one or more path segments | url userdata | Append path segments                     |
| `:with_extension` | `ext: string`             | url userdata | Replace extension (leading `.` optional) |
| `:relative_to`    | `base` (url or string)    | `string`     | Relative path from `base`                |
| `__tostring`      | —                         | `string`     | Stored raw form                          |

---
