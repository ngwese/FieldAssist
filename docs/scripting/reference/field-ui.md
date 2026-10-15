# `field.ui`

<!-- markdownlint-disable MD013 MD060 -->
Toolbar control constructors (data tables, not GPUI widgets) plus RGBA
palettes. In FieldAssist, `field.ui.named` and `field.ui.semantic` are
overridden after the shared namespace is bound, so every named and semantic
color is read from the live GPUI theme. Headless hosts use the portable
defaults.

## field.ui Properties
| Property   | Access | Type             | Description                                                   |
| ---------- | ------ | ---------------- | ------------------------------------------------------------- |
| `named`    | **ro** | palette userdata | Live GPUI named colors in FieldAssist; defaults headlessly    |
| `semantic` | **ro** | palette userdata | Live GPUI semantic colors in FieldAssist; defaults headlessly |

## field.ui Functions
| Function     | Arguments        | Returns       | Description                     |
| ------------ | ---------------- | ------------- | ------------------------------- |
| `button`     | `props: table`   | control table | Push-button                     |
| `toggle`     | `props: table`   | control table | On/off toggle                   |
| `message`    | `props: table`   | control table | Static / updatable text         |
| `text_entry` | `props: table`   | control table | Single-line text field          |
| `path_entry` | `props: table`   | control table | Path field with optional browse |
| `select`     | `props: table`   | control table | Dropdown of fixed choices       |
| `divider`    | `props` or `nil` | control table | Visual separator                |
| `progress`   | `props: table`   | control table | Sheet-only progress bar/circle  |
| `log`        | `props: table`   | control table | Sheet-only scrolling log        |

Controls are tables with a control metatable and a `kind` field. Optional
`action = function(control, workflow)` runs on click / commit. Assigning
watched props (`label`, `value`, `text`, `color`, `on_color`, `off_color`,
`align`, `icon`, `on_icon`, `choices`, `enabled`, `loading`, `variant`)
refreshes the host toolbar or sheet. `progress` and `log` are rejected by
`:set_toolbar`.

## Returned type: control table
### Common properties
| Property | Access | Type                 | Description                         |
| -------- | ------ | -------------------- | ----------------------------------- |
| `kind`   | **ro** | `string`             | Set by the constructor              |
| `id`     | **rw** | `string`             | Stable id (required for most kinds) |
| `align`  | **rw** | `"left"` \| `"right"` | Toolbar / button-bar alignment (default left) |
| `enabled`| **rw** | `boolean`            | Interactive controls (default true) |
| `action` | **rw** | `function` or `nil`  | `function(control, workflow)`       |

### Per-kind properties
| Kind           | Properties                                                                                                                                                                   |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **button**     | `id?`, `label?` (one of id/label required), `icon?` (`check`, `circle-check` / `circle_check`, `circle-x`, `circle-alert`, `arrow-left`, `arrow-right`), `color?` (sheet footer accent, e.g. `app.theme.semantic.danger`), `align?`, `enabled?`, `action?` |
| **toggle**     | `id` (required), `label?` (defaults to id), `value?` bool (default `false`), `on_color?`, `off_color?`, `on_icon?` (default `"check"`), `align?`, `enabled?`, `action?`                  |
| **message**    | `id` (required), `text` (required string; may be `""`), `color?`, `align?`                                                                                                   |
| **text_entry** | `id` (required), `label?`, `value` (string, default `""`), `align?`, `enabled?`, `action?`                                                                                               |
| **path_entry** | `id` (required), `label?`, `value` (string), `browse?` (`"file"` \| `"directory"` \| `true` (=directory) \| `false` \| `nil`), `align?`, `enabled?`, `action?`                               |
| **select**     | `id` (required), `label?`, `value` (selected choice id, default `""`), `choices` (array of strings or `{ value, label? }`), `align?`, `enabled?`, `action?`                                 |
| **divider**    | `id?`, `align?`                                                                                                                                                              |
| **progress**   | `id` (required), `label?`, `variant?` (`"bar"` default \| `"circle"`), `value?` (0–100), `loading?` bool, `text?` caption. Omit `value` or set `loading` for indeterminate |
| **log**        | `id` (required), `label?`, `text?`. Method `:append(line)` appends a line (cap 1000)                                                                                          |

## Palette: `field.ui.named`
| Property                                                                                                                                     | Access | Type             |
| -------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ---------------- |
| `red`, `red_light`, `green`, `green_light`, `blue`, `blue_light`, `yellow`, `yellow_light`, `magenta`, `magenta_light`, `cyan`, `cyan_light` | **ro** | `{ r, g, b, a }` |

## Palette: `field.ui.semantic`
| Property                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Access | Type             |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------ | ---------------- |
| `accent`, `accent_foreground`, `background`, `border`, `danger`, `danger_active`, `danger_foreground`, `danger_hover`, `drop_target`, `foreground`, `info`, `info_active`, `info_foreground`, `info_hover`, `input`, `link`, `link_active`, `link_hover`, `muted`, `muted_foreground`, `popover`, `popover_foreground`, `primary`, `primary_active`, `primary_foreground`, `primary_hover`, `ring`, `secondary`, `secondary_active`, `secondary_foreground`, `secondary_hover`, `selection`, `success`, `success_active`, `success_foreground`, `success_hover`, `warning`, `warning_active`, `warning_foreground`, `warning_hover`, `chart_1`…`chart_5`, `chart_bullish`, `chart_bearish` | **ro** | `{ r, g, b, a }` |

---
