# FieldAssist workflows specification

**Document status:** Provisional

### Revision history

| Revision | Date | Notes |
| --- | --- | --- |
| 1 | 2026-09-11 | Initial as-built specification |
| 2 | 2026-09-12 | Prototype/instance pattern; `app:run_workflow` |

This document specifies the Lua host, workflow system, and how they support
incremental review. The Lua surface is defined in [SCRIPT.md](../SCRIPT.md);
this file is the product contract.

Related:

- [SPEC-application.md](SPEC-application.md) — session, composition, UI
- [SPEC-processing.md](SPEC-processing.md) — future processing chains (not
  driven by these workflows today)
- [SCRIPT.md](../SCRIPT.md) — API reference

## Purpose

FieldAssist embeds Lua 5.4 so ingest, layout detection, and multi-file
review can be scripted instead of hard-coded. The original product tracked
“what has been reviewed” on a workspace collection. The as-built app does
that with **session document groups** plus a **stateful Review workflow**,
not a `review_status` enum or a database.

Scripts do not host plugins and do not run processing chains. They open
documents, set `group` / `state` / `properties`, fire commands, and bind a
workflow to the session.

## Runtime

There is one host-provided global besides Lua’s standard libraries: `app`.
`print` writes to the Script panel. `app:info`, `app:warn`, and `app:error`
write to Messages (and to stdout when it is a terminal). Times and channel
indices are 0-based samples.

### Config directory

| OS | Directory |
| --- | --- |
| macOS | `~/Library/Application Support/FieldAssist/` |
| Windows | `%APPDATA%\FieldAssist\` |
| Linux | `$XDG_CONFIG_HOME/FieldAssist/` or `~/.config/FieldAssist/` |

`FieldAssist --dump-init` prints the **embedded** default `init.lua`.

### Load order at startup

1. Embedded `workflow_add.lua`, `workflow_replace.lua`, `workflow_review.lua`
2. `init.lua`: the user file in the config directory if it exists, otherwise
   the embedded default
3. User `workflow_*.lua` next to `init.lua`, sorted by path

`app:declare_workflow` with an existing `name` replaces the previous
registration. Declaring during an in-progress drag does not rebuild that
gesture’s overlay.

Embedded `init.lua` defines layouts, `detect_layout`, and `loaded` / `saved`
logging. It does not register `session_loaded` / `session_saved`; those hooks
exist for user scripts.

## Session and documents from Lua

`app.session` is always the UI-active session. `app.composition` /
`app.compositions` / `app:open` alias `session.composition` / `session.compositions` /
`session:open`. `app.workflow` is the running stateful workflow instance, or
`nil`.

- Audio and `.facomp` **add** a document
- `session:open` of a `.fasession` **replaces** the active session (same as
  File → Open / CLI)
- `app:load_session` loads a `.fasession` into memory without replacing the
  UI session (used by Add to merge paths)

Document `group`, `state`, and `properties` persist in the `.fasession`.
They are not written to `.facomp`. Built-in Review uses **`group` only**
(`todo`, `reviewed`, `drop`). `state` is available to other scripts.

`session.properties` is a string→string map (`nil` values rejected). Review
stores its output directory as `properties.output` on suspend.

`session.workflow_name` is the bound stateful workflow name, persisted in the
`.fasession`. The host writes `instance:name()` after a successful `:start`
on a stateful instance. The running instance is exposed as `app.workflow`.
Scripts should end a run with `app:finish_workflow()` /
`app:cancel_workflow()`, not by assigning the name field. Assigning the name
alone does not show a toolbar; that requires `:start` or `:resume`.

## Workflows

A workflow is a named prototype. Each run is a new Lua instance (metatable =
prototype). Optional drop layout, menu presence, lifecycle methods, and a
toolbar live on that instance.

| Kind | Rule |
| --- | --- |
| **One-shot** | No `:suspend` or `:resume`. `:start` runs and returns. Does not bind `session.workflow_name` or `app.workflow`. No toolbar |
| **Stateful** | Defines `:suspend` and/or `:resume`. After `:start`, the host keeps the instance as `app.workflow`, binds `session.workflow_name` to `instance:name()`, and may show a toolbar |

`app:create_workflow(props)` builds a prototype with `__base_properties`
userdata and base methods (`:name()`, `:on`, `:set_toolbar`, …).
`app:declare_workflow` registers it. `app:declare_workflow(props, func)` is
shorthand for a one-shot whose `:start` is `func(payload)` (no `self`).
`app:run_workflow(name)` / `app:run_workflow(name, payload)` look up the
prototype, construct an instance (`:init` if defined), and call `:start`.
A bare name uses `{ scope = "run" }`.

**Scopes:** `"drag-drop"` and/or `"menu"`.

**Start payloads:**

- Drop overlay: `{ scope = "drag-drop", paths = { ... } }`
- Workflow menu: `{ scope = "menu" }` (no `paths`)
- `app:run_workflow(name)`: `{ scope = "run" }`

**Concurrency:** one stateful workflow per active session. Starting a
*different* stateful workflow while one is bound alerts and does not start.
Dropping the *same* running workflow creates a new instance and calls
`:start` again. One-shot Add and Replace may run while Review is bound.

**Session replace:** cancel the outgoing workflow, install the new session,
fire `session_loaded`, then construct a new instance and call `:resume` if
the incoming `session.workflow_name` names a declared stateful prototype.
Unknown or one-shot names: log, no toolbar, Keep (leave the name) or Clear
(unbind only; no `:cancel` / `:finish`; other properties stay).

**Suspend** runs before Save Session / Save Session As, and during quit or
open-session after compositions are clean (before the unsaved-session
prompt). Return `false` (or error) aborts that save or quit.

### Drop overlay

Drag over the center waveform or empty pane. Layout is computed **once** for
the gesture from workflows whose scopes include `"drag-drop"`:

- Rows keyed by `drop.row` (missing numbers are not padded)
- Within a row: higher `drop.priority` first (left); ties by `display_name`
  then `name`
- Box width is `priority / sum(priorities in the row)`

Built-ins: row 1 Add and Replace (equal priority → equal widths, Add then
Replace by name); row 2 Review.

### Workflow menu

After View: **Cancel** (disabled when none is running), then a divider, then
every workflow whose scopes include `"menu"`, labeled with `display_name`,
sorted alphabetically.

### Toolbar

`:set_toolbar(items)` / `:set_item(id, props)` from `start`, `resume`, or a
workflow-local command handler. The bar sits between the dock and the status
bar while a stateful workflow is bound and items are non-empty. It shows
`display_name`, left-aligned items, a spacer, then right-aligned items.

Item kinds: `button` (or omitted when `command` is set), `path`, `toggle`,
`message`, `divider`. Workflow `command` strings are **not** keymap ids.
`:on("command", handler)` is workflow-local; last registration wins.

## Built-in workflows

### Add (one-shot, drag-drop)

For each dropped path:

- `.fasession` → `load_session`, `session:open` each incoming document path
  not already in the active session, then close the loaded session
- otherwise → `session:open(path)`

### Replace (one-shot, drag-drop)

Exactly one path required; otherwise an alert.

- `.fasession` → `session:open` (replace the UI session)
- else if there is an active composition → `active:replace(path)` (not a
  session file)
- else → `session:open(path)`

### Review (stateful, drag-drop and menu)

Review is the as-built stand-in for “work through the collection.”

**Groups:** `todo`, `reviewed`, `drop` on `c.group`.

**Start:**

- `menu`: every open session document → `todo`
- `drag-drop`: skip `.fasession`; expand directories with `app:find_files`
  for readable audio / `.facomp` extensions; open each file into `todo`

Start and resume turn transport loop and Preview on, show the explorer, and
install the toolbar: Previous, Next, Drop, Reviewed toggle, progress
message, Output directory, Finish.

- Previous / Next cycle `todo` (wrap)
- Drop moves the active document to `drop`
- Reviewed on → `reviewed` and advance like Next if other todos remain;
  off → `todo`
- Progress is `{reviewed} of {reviewed + todo} files reviewed`
- Finish → `app:finish_workflow()`; warns if any documents remain `todo`;
  turns loop and Preview off
- `suspend` writes `session.properties.output`; `resume` restores it

## Layouts and detect

`app:define_layout` registers a named channel layout and optional default
`monitor = { chain = "…" }`. Embedded layouts:

| Name | Default monitor |
| --- | --- |
| `mono` | `mono` |
| `stereo` | `stereo` |
| `MS` | `ms` |
| `B-Format (AmbiX)` | `foa` |
| `B-Format (FuMa)` | `foa_fuma` |
| `2OA` | none |

`detect_layout` is `function(c, chosen)`. `chosen` is a persisted
user-explicit name or `nil`. Hooks run in registration order; the last
valid defined name wins. Embedded detection remaps `"1OA"` to
`B-Format (AmbiX)`, then honors `chosen`, then basename keywords (`fuma`
before `ambix` when `n >= 4`), then channel-count fallbacks (1, 2, 4, 9).

Detecting a layout fills `monitor_chain` only when it is still unset.
Monitor DSP remains the listen path; workflows do not implement processing
chains.

## Hooks

| Event | Signature |
| --- | --- |
| `loaded` | `(composition, elapsed_seconds)` |
| `saved` | `(composition, elapsed_seconds)` |
| `session_loaded` | `(session)` |
| `session_saved` | `(session)` |
| `detect_layout` | `(composition, chosen) → name or nil` |

Unknown `app:on` names are errors. Hook errors from `loaded` / `saved` print
to the Script panel.

## Commands from scripts

`app:command(id)` runs the same ids as menus and the keymap (see
[SPEC-application.md](SPEC-application.md)). Unknown ids error. Workflow
toolbar commands are a separate namespace.

Scripts can open, replace, save, and close compositions; save the active
session; edit markers, regions, collections, and the EDL; set layout and
monitor fields; and log or alert. They cannot register new keymap items,
close the UI session object, edit a composition that is only referenced by a
detached `load_session`, or start a second stateful workflow.

## Processing

Workflows may later *invoke* batch export or assign a processing chain to
documents (see [SPEC-processing.md](SPEC-processing.md)). Nothing in the
current host does that. Review’s Output path is stored for scripts; the app
does not yet render into it automatically.
