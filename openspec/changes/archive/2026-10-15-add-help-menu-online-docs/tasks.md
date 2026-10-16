# Tasks

## 1. Command surface

- [x] 1.1 Add a `HelpOnlineDocs` unit action to the `actions!` list in
      `crates/field-assist/src/commands.rs`, mirroring the existing `About`
      entry; verify `cargo build -p FieldAssist` compiles.
- [x] 1.2 Add `"help.online_docs"` to `KNOWN_COMMANDS` in the same file next
      to `"help.about"`; extend or add a unit test asserting
      `is_known_command("help.online_docs")` is true and that an unknown id
      still returns false; verify
      `cargo test -p FieldAssist known_command` passes.

## 2. Menu wiring

- [x] 2.1 Add a helper in `crates/field-assist/src/app.rs` that builds the
      shared `Help` menu items (`FieldAssist Help...`, separator, `About...`)
      and use it for the non-macOS `Help` menu; verify
      `cargo build -p FieldAssist` compiles.
- [x] 2.2 Push that `Help` menu on macOS after the `Workflow` menu in
      `app_menus()`, keeping the existing non-macOS position; verify the
      macOS menu-building code path compiles and a test over `app_menus()`
      (or the helper) asserts `FieldAssist Help...` precedes `About...` and
      follows the separator; run `cargo test -p FieldAssist` and confirm it
      passes.

## 3. Open the documentation site

- [x] 3.1 Intercept `"help.online_docs"` in `app::dispatch_command` next to
      `help.about` and call `cx.open_url("https://ngwese.github.io/field-docs/")`;
      verify `cargo build -p FieldAssist` compiles with no new warnings.
- [x] 3.2 Add a unit test for the constant URL and its id routing so the
      documented site string is asserted in one place; verify
      `cargo test -p FieldAssist` passes.

## 4. Documentation

- [x] 4.1 Add `help.online_docs` to the **Help:** command list in
      `docs/spec/SPEC-application.md` beside `help.about`, and note the
      `Help` menu now appears on macOS; verify the file lists both ids and
      reads consistently with the implementation.

## 5. Integration checks

- [x] 5.1 Run `cargo fmt`, then `cargo build` and `cargo test` across the
      workspace; confirm no errors and no warnings (including unused imports
      and dead code).
- [ ] 5.2 Launch the app on the current platform and confirm the `Help` menu
      shows `FieldAssist Help...` first, that choosing it opens the docs site
      in the default browser, and that the app window stays open.

## Workflow follow-up

- Archive the change after review is satisfied; confirm the new
  `openspec/specs/application-menu/spec.md` is created from the delta.
