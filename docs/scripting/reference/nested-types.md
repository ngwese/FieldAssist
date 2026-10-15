# Nested types

<!-- markdownlint-disable MD013 MD060 -->
## collection
Region list returned by `composition.selection` or `composition:collection(name)`.

### collection Properties
| Property  | Access | Type            | Description                |
| --------- | ------ | --------------- | -------------------------- |
| `name`    | **ro** | `string`        | `"selection"` or user name |
| `regions` | **ro** | `{ region, … }` | Regions in the collection  |

`#collection` is the region count.

### collection Methods
(none)

## region
### region Properties
| Property     | Access | Type                    | Description                           |
| ------------ | ------ | ----------------------- | ------------------------------------- |
| `id`         | **ro** | `integer`               | Region id                             |
| `start`      | **ro** | `integer`               | Start sample                          |
| `stop`       | **ro** | `integer`               | End sample (inclusive span as stored) |
| `channels`   | **ro** | `"all"` or `{ int, … }` | Channel mask                          |
| `label`      | **ro** | `string` or `nil`       | Optional label                        |
| `collection` | **ro** | `string`                | Owning collection name                |

### region Methods
(none)

## marker
