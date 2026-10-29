# Spec Delta

## Purpose

Defines the shared Lua `app.output_device` preference for headless hosts so
init scripts can pin a playback device the same way FieldAssist already
allows, and so field-play honors that preference when opening audio.

Informed by `docs/scripting/reference/app.md` and the field-play init path in
`docs/scripting/guide.md`.

## ADDED Requirements

### Requirement: Shared app exposes output_device

field-play and field-batch SHALL expose `app.output_device` as a read/write
Lua field of type string or nil. A non-string, non-nil assignment MUST raise
a Lua runtime error. Assigning a string MUST store that preference without
opening a CPAL stream. Assigning nil MUST clear the preference.

#### Scenario: Init sets preferred device

- **WHEN** an `init.lua` loaded by field-play or field-batch assigns
  `app.output_device = "Focusrite"`
- **THEN** the assignment succeeds and a subsequent read of
  `app.output_device` returns `"Focusrite"`

#### Scenario: Nil clears preference

- **WHEN** a script assigns `app.output_device = nil`
- **THEN** a subsequent read of `app.output_device` returns nil

#### Scenario: Invalid type rejected

- **WHEN** a script assigns a non-string, non-nil value to
  `app.output_device`
- **THEN** Lua raises a runtime error and the stored preference is unchanged

### Requirement: Shared app exposes output_devices

field-play and field-batch SHALL expose read-only `app.output_devices` as a
Lua array of known output device name strings (empty when enumeration fails).

#### Scenario: List known devices

- **WHEN** a script reads `app.output_devices` on a host that can enumerate
  outputs
- **THEN** the value is a 1-based array of device name strings

### Requirement: Settings seed the preference before init

Before loading `init.lua`, field-play and field-batch SHALL seed
`app.output_device` from `settings.json` `audio.output_device` when that
value is present. An assignment in `init.lua` MUST override the seeded
value for the remainder of the process. Writes to `app.output_device` MUST
NOT persist into `settings.json`.

#### Scenario: Settings then init override

- **GIVEN** `settings.json` contains `"audio": { "output_device": "Speakers" }`
- **WHEN** field-play or field-batch loads that config and `init.lua` sets
  `app.output_device = "Headphones"`
- **THEN** the effective preference after init is `"Headphones"` and
  `settings.json` is unchanged

#### Scenario: Settings alone

- **GIVEN** `settings.json` contains `"audio": { "output_device": "Speakers" }`
  and `init.lua` does not assign `app.output_device`
- **WHEN** field-play or field-batch finishes loading init
- **THEN** the effective preference is `"Speakers"`

### Requirement: field-play resolves the post-init preference

When opening the playback stream, field-play SHALL resolve the CPAL output
device from the effective `app.output_device` preference after init (nil
means system default), using the same name / index / substring matching as
existing output-device resolution. Resolution and stream open remain off the
CPAL output callback.

#### Scenario: Preference selects device

- **GIVEN** after init the effective preference is a non-nil substring that
  matches exactly one output device
- **WHEN** field-play opens playback
- **THEN** it opens that matched device

#### Scenario: Nil uses default

- **GIVEN** after init the effective preference is nil
- **WHEN** field-play opens playback
- **THEN** it opens the system default output device
