# Proposal

## Why

FieldAssist has offline product documentation at
<https://ngwese.github.io/field-docs/>, but the desktop app offers no way to
reach it. Users must leave the app and find the site themselves. The Help
menu is also absent on macOS: today macOS only carries `About...` inside the
app menu, while Windows and Linux get a `Help` menu whose sole item is
`About...`.

## What Changes

- Add a `FieldAssist Help...` menu item to the `Help` menu, placed as its
  first entry, above `About...` and a separator.
- Introduce the `Help` menu on macOS. It sits after `Workflow`, matching the
  position the menu already occupies on Windows and Linux.
- Choosing `FieldAssist Help...` opens the system default browser to
  <https://ngwese.github.io/field-docs/> using the platform's URL-opening
  mechanism.
- Add a stable command id (`help.online_docs`) so the keymap, menu, and Lua
  `app:command` continue to share one command vocabulary.

## Capabilities

### New Capabilities

- `application-menu`: the desktop app's menu bar and command surface — which
  menus exist on which platforms, the items they contain and their order, and
  how a menu item maps to a command id and an action.

### Modified Capabilities

<!-- No existing capability's requirements change. The three current specs
     (catalog-workflow, composition-render, export-tags) do not cover the menu
     bar or command surface. -->

## Impact

- **Affected host**: FieldAssist (GPUI desktop app) only. `field-batch` and
  `field-play` are headless and have no menu bar; they are unaffected.
- **Affected crates**: `field-assist` owns the menu definition, the command
  table, and the action handlers. No change is expected in
  `field-ui-components`, which renders menus it is handed rather than defining
  their contents.
- **Dependencies**: no new crate dependency. The browser launch uses the
  URL-opening entry point already provided by the GPUI platform layer, which
  dispatches to the native mechanism per OS (NSWorkspace on macOS, `xdg-open`
  on Linux, shell-open on Windows).
- **Docs**: the command list in `docs/spec/SPEC-application.md` gains the new
  `help.online_docs` id alongside `help.about`.
- **Realtime risk**: none. Menu handling and browser launch run on the UI
  thread and never touch the CPAL output callback or the monitor/prefetch
  audio paths.

## Non-goals

- No in-app documentation viewer, embedded web view, or offline copy of the
  docs site.
- No changes to `About...` behavior, placement, or the About window.
- No Help menu on macOS beyond the two items this change introduces; no
  search/help index, keyboard-shortcut reference, or "Report an Issue" item.
- No new keybinding for the online-docs command; the item is menu-only.
- No change to how Windows and Linux order or render their existing `Help`
  menu other than adding the new first item.
