# Design

## Context

See proposal.md - Why. The relevant current state:

- `app_menus()` in `crates/field-assist/src/app.rs` builds the platform menu
  list. macOS gets an app menu (`About...`, `Settings...`, `Install CLI
  Tools`, Hide/Quit, ...) and no `Help` menu today; Windows and Linux get a
  `Help` menu whose only item is `About...`.
- Menu items are declared with `MenuItem::action(label, Action)`. The action
  is a `gpui_kit::actions!` unit struct in `crates/field-assist/src/commands.rs`.
- Actions funnel into `commands::dispatch("domain.name", cx)`, which validates
  the id against `KNOWN_COMMANDS`, optionally intercepts Lua-registered
  overrides via `script::try_invoke_command`, and otherwise calls
  `app::dispatch_command`.
- `app::dispatch_command` handles non-window commands inline, e.g.
  `help.about` calls `open_about_window(cx)`.
- GPUI exposes `App::open_url(&self, url: &str)`, re-exported through
  `gpui_kit::*`. Its platform backends launch the OS default handler
  (NSWorkspace on macOS, `xdg-open` on Linux, shell-open on Windows). No new
  crate dependency is needed.

## Goals / Non-Goals

**Goals:**

- Add the `Help` menu on macOS and put `FieldAssist Help...` first in it on
  all platforms.
- Route the item through the same action → command-id → dispatch path the
  other menu items use, so it stays consistent with the keymap and Lua
  `app:command`.
- Keep the browser launch on the UI thread with no new dependency.

**Non-Goals:**

- No change to `About...`, no keybinding, no in-app docs viewer (see
  proposal.md - Non-goals).
- No new abstraction or trait for "open a URL"; the platform entry point is
  already the right seam.

## Decisions

### Use `App::open_url` for the browser launch

`App::open_url` is the platform-neutral seam GPUI already provides and is
already reachable via `gpui_kit::*`. On every desktop target it hands the URL
to the OS default handler, which is exactly the required behavior.

Alternatives considered:

- **Add an `open`/`webbrowser` crate.** Rejected: a new dependency duplicates
  what the platform layer already does, and `webbrowser` shells out or
  spawns helpers on some targets where GPUI uses the native API.
- **Call `NSWorkspace` directly behind `#[cfg(target_os = "macos")]`, with
  `xdg-open` elsewhere.** Rejected: more code, more failure modes, and it
  bypasses the platform abstraction the crate already uses.

### Declare the action as a unit action and register a command id

Add `HelpOnlineDocs` to the `actions!` list and `"help.online_docs"` to
`KNOWN_COMMANDS`, consistent with `About` / `"help.about"`. Intercept it in
`dispatch_command` next to `help.about`.

Alternatives considered:

- **Handle it only in the menu closure.** Rejected: the menu, keymap, and Lua
  `app:command` are meant to share one id space (see
  `docs/spec/SPEC-application.md` - Commands); registering the id keeps that
  invariant and makes the action scriptable without extra work.

### Define the Help menu list once for all platforms

Introduce a small helper that returns the `Help` menu items
(`FieldAssist Help...`, separator, `About...`) and use it both for the macOS
menu insertion and the existing non-macOS `Help` menu, so the two platforms
cannot drift. On macOS the menu is pushed after `Workflow`; on Windows and
Linux it keeps its current position at the end of the list.

Alternatives considered:

- **Duplicate the macOS and non-macOS item lists.** Rejected: two copies of
  the same order invite drift when items are added later.

## Risks / Trade-offs

- [Non-macOS `Help` menu must keep `About...`] → The shared helper includes
  `About...`, so Windows and Linux keep the item they have today; only the
  new first item is added.
- [macOS gains a `Help` menu that previously did not exist] → Intended, and
  specified: the menu contains only the two items this change defines, so it
  cannot surprise users with unrelated entries.
- [`open_url` failure is not surfaced] → The platform call returns no error.
  Consistent with the existing menu behavior, a failed launch is silent; no
  extra error UI is added.
- [Realtime audio] → None. Nothing in this change runs on the CPAL output
  callback or the prefetch path; it is UI-thread menu handling only.

## Migration Plan

Not applicable. This is an additive UI change with no persisted state, no data
migration, and no rollback concerns; reverting the commit removes the item.
