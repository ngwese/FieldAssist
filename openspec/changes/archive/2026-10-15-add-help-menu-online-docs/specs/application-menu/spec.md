# Spec Delta

## Purpose

Defines the FieldAssist desktop menu bar and its command surface: which
menus exist on which platforms, the items they contain and their order, and
how choosing an item maps to a shared command id.

## ADDED Requirements

### Requirement: Help menu is present on every desktop platform

The FieldAssist desktop app SHALL present a top-level `Help` menu on macOS,
Windows, and Linux. On macOS the `Help` menu SHALL appear after the `Workflow`
menu, matching the position it occupies on the other platforms.

#### Scenario: Help menu on macOS

- **WHEN** the FieldAssist desktop app runs on macOS
- **THEN** the menu bar contains a `Help` menu positioned after `Workflow`

#### Scenario: Help menu on Windows and Linux

- **WHEN** the FieldAssist desktop app runs on Windows or Linux
- **THEN** the menu bar contains a `Help` menu

### Requirement: Help menu lists online docs first

The `Help` menu SHALL contain a `FieldAssist Help...` item as its first entry.
The `FieldAssist Help...` item SHALL be enabled whenever the menu bar is
available. On Windows and Linux the item SHALL be followed by a separator and
the existing `About...` item. On macOS the `Help` menu SHALL NOT contain
`About...`, because that item remains in the application menu.

#### Scenario: Help menu item order on Windows and Linux

- **WHEN** the user opens the `Help` menu on Windows or Linux
- **THEN** `FieldAssist Help...` is the first item
- **AND** it is followed by a separator and then `About...`

#### Scenario: Help menu on macOS omits About

- **WHEN** the user opens the `Help` menu on macOS
- **THEN** `FieldAssist Help...` is the only item
- **AND** `About...` is available from the application menu

### Requirement: Help menu opens the product documentation site

Choosing the `FieldAssist Help...` item SHALL open the system default browser
to `https://ngwese.github.io/field-docs/`. The app SHALL use the platform's
native URL-opening mechanism and SHALL NOT navigate away from the application
or block the UI while launching the browser.

#### Scenario: Browser opens the documentation site

- **WHEN** the user chooses `FieldAssist Help...`
- **THEN** the system default browser opens `https://ngwese.github.io/field-docs/`

#### Scenario: App stays in place

- **WHEN** the user chooses `FieldAssist Help...`
- **THEN** the FieldAssist window remains open and responsive

### Requirement: Online docs command shares the common command id space

The online-docs action SHALL be addressable by the command id
`help.online_docs`, alongside the existing `help.about`. The id SHALL be
accepted by the same dispatch path used by menus, the keymap, and Lua
`app:command`.

#### Scenario: Command id is recognized

- **WHEN** the command id `help.online_docs` is dispatched
- **THEN** the system default browser opens the documentation site

#### Scenario: Unknown command ids still fail

- **WHEN** a command id that is not in the known-command set is dispatched
- **THEN** dispatch reports an unknown-command error and opens nothing
