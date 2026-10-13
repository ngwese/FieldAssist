# Spec Delta

## Purpose

Catalog exports kept field-recording compositions into a library directory
suitable for sample browsers such as Sononym, then optionally cleans staging.

## ADDED Requirements

### Requirement: Catalog source set is keep group

The Catalog workflow SHALL treat every session document with
`group == "keep"` as an export source, including child compositions and
trimmed parents. Documents in any other group SHALL NOT be exported.

#### Scenario: Keep compositions are cataloged

- **GIVEN** a session with documents in `keep`, `drop`, and `todo`
- **WHEN** Catalog Run executes after a successful preflight
- **THEN** only `keep` documents are rendered to the library

#### Scenario: Child composition is its own export

- **GIVEN** a kept child composition that isolates a region or channel subset
- **WHEN** Catalog exports that document
- **THEN** the exported media reflects that composition's edit and name

### Requirement: Library path and export profile variables

The system SHALL resolve the library root from session variable
`catalog.library_root`, falling back to `user.catalog.library_root` when
unset or empty. The system SHALL resolve the default export profile name
from `catalog.export_profile`, defaulting to `FLAC (Source Equivalent)`
when unset. The Catalog sheet SHALL offer a pulldown that overrides the
profile for the current run without requiring a persistent write.

#### Scenario: Fallback library root

- **GIVEN** `catalog.library_root` is empty and `user.catalog.library_root`
  is set
- **WHEN** Catalog configures destinations
- **THEN** outputs are planned under `user.catalog.library_root`

#### Scenario: Profile pulldown override

- **GIVEN** `catalog.export_profile` names a profile
- **WHEN** the user selects a different profile in the Catalog sheet
- **THEN** Run uses the selected profile for that batch

### Requirement: Destination layout mirrors staging event dirs

Catalog SHALL place each identity export at
`{library_root}/{event}/{composition_name}.{profile_extension}` where
`event` is `source.event.name` for the composition's media when present;
otherwise the media (or composition) parent folder basename — so a
reloaded session still mirrors staging event dirs even though script
`source.*` bindings are not persisted. Omit the event segment only when
no safe name is available. `composition_name` is the document's current
name. Renaming a composition SHALL change the planned output basename.

#### Scenario: Renamed composition

- **GIVEN** a kept composition renamed to `dawn-chorus`
- **WHEN** Catalog plans destinations
- **THEN** the identity file basename is `dawn-chorus` with the profile
  extension

#### Scenario: Event directory carried over

- **GIVEN** `source.event.name` is `2026-10-04`
- **WHEN** Catalog exports the composition
- **THEN** the file is written under `{library_root}/2026-10-04/`

#### Scenario: Event directory after session reload

- **GIVEN** staging media under `{staging_root}/26-09-27/` and a reloaded
  `.fasession` whose in-memory `source.event.name` is unset
- **WHEN** Catalog plans destinations
- **THEN** outputs are still placed under `{library_root}/26-09-27/`

### Requirement: Project files copy beside rendered samples

Catalog SHALL copy each kept composition's `.facomp` (when the document is
backed by one) into the same library event directory as its rendered
samples. Catalog SHALL also copy the focused `.fasession` into each library
event directory that received rendered samples. These copy destinations
SHALL participate in preflight conflict checks. Existing destinations at
copy time SHALL be skipped with a warning rather than overwritten.

#### Scenario: Facomp copied next to renders

- **GIVEN** a kept composition saved as `take.facomp` under staging
- **WHEN** Catalog renders that composition into `{library}/{event}/`
- **THEN** `{library}/{event}/take.facomp` is created by copy

#### Scenario: Session copied into event dirs

- **GIVEN** a focused `review.fasession` and successful renders under
  `{library}/26-09-27/`
- **WHEN** Catalog Run completes
- **THEN** `{library}/26-09-27/review.fasession` exists

### Requirement: Stereo sidecar naming uses layout code to st

When Catalog emits a stereo mixdown sidecar, the output basename SHALL be
derived from the identity basename by replacing the composition's layout
**code** token with `st` for codes `ambix`, `fuma`, and `ms`. Mono,
stereo, and 2OA exports SHALL NOT produce a mixdown sidecar.

#### Scenario: AmbiX sidecar name

- **GIVEN** a kept FOA AmbiX composition whose name includes layout code
  `ambix`
- **WHEN** Catalog plans outputs
- **THEN** the sidecar basename uses `st` in place of `ambix`

#### Scenario: FuMa and M/S use the same rule

- **GIVEN** kept compositions with layout codes `fuma` or `ms`
- **WHEN** Catalog plans mixdown outputs
- **THEN** each sidecar basename replaces that code with `st`

#### Scenario: 2OA has no mixdown

- **GIVEN** a kept composition with layout `2OA` (no monitor mixdown chain)
- **WHEN** Catalog exports
- **THEN** only the identity file is produced

### Requirement: Preflight catches destination conflicts

Before Run starts encoding, Catalog SHALL build the full output path plan
for all keep sources and SHALL fail the preflight (blocking Run) when any
planned destination already exists on disk or when two planned outputs
collide. During render, Catalog SHALL re-check each destination; if a
conflict appears, that output SHALL be skipped and reported in the run
summary.

#### Scenario: Preflight fails on existing file

- **GIVEN** a planned library path already exists
- **WHEN** the user attempts Run
- **THEN** preflight fails with a clear conflict list and no encode starts

#### Scenario: Preflight fails on colliding keeps

- **GIVEN** two keep compositions that resolve to the same destination path
- **WHEN** preflight runs
- **THEN** preflight fails and Run does not start

#### Scenario: Mid-run conflict is skipped

- **GIVEN** a destination appears after a successful preflight
- **WHEN** that output is about to be written
- **THEN** Catalog skips it and includes the skip in the summary

### Requirement: Sheet UX with summary and staging cleanup

Catalog SHALL present a modal sheet flow: configure (library root, profile),
preflight/run progress, summary (ok / skipped / failed), then a cleanup
page. Advancing Cleanup SHALL open a native FieldAssist confirm dialog
(`app:confirm`) whose OK action is danger-styled and labeled for cleanup;
staging deletion SHALL run only from that dialog's confirm callback (not
from a synchronous true default). On confirm, the system SHALL delete
every session-referenced file whose path lies under the session staging
root, including `drop` and never-exported media, and the focused
`.fasession` itself when it lies under staging. Relative media /
composition URLs SHALL be resolved against the session directory before
the staging-root check. Open documents SHALL be force-closed (discard
unsaved; no save prompt) before unlinking so deletes succeed.
Paths outside the staging root SHALL NOT be deleted. After those files are
removed, Cleanup SHALL also remove any parent directories under the staging
root that become empty (staging root itself is retained). When empty
parent directories will be removed, the cleanup summary SHALL note both the
file count and directory count (e.g. `N file(s) under staging and M
directories`). Cleanup SHALL remain available after partial export
failure, and the cleanup page SHALL warn when any export failed or was
skipped.

#### Scenario: Native confirm before staging wipe

- **GIVEN** the Catalog cleanup page lists staging files to remove
- **WHEN** the user activates Cleanup
- **THEN** a native confirm dialog appears and no staging files are
  deleted until the user confirms in that dialog

#### Scenario: Cleanup includes drops under staging

- **GIVEN** session documents in `keep` and `drop` whose media paths are
  under `staging_root`
- **WHEN** the user confirms cleanup after Catalog Run
- **THEN** both keep and drop staging files are deleted

#### Scenario: Cleanup includes session file under staging

- **GIVEN** a focused `.fasession` stored under `staging_root` (e.g.
  `{staging}/26-09-27/review.fasession`)
- **WHEN** the user confirms cleanup
- **THEN** the session file is deleted along with session-referenced
  staging media and compositions

#### Scenario: Outside staging left alone

- **GIVEN** a session document path outside `staging_root`
- **WHEN** cleanup runs
- **THEN** that path is not deleted

#### Scenario: Empty parent directories under staging are removed

- **GIVEN** session-referenced staging files live under
  `{staging_root}/{event}/…` and no other files remain in `{event}` after
  cleanup
- **WHEN** the user confirms cleanup
- **THEN** those files and the empty `{event}` directory are deleted, the
  staging root itself remains, and the cleanup summary reports both file
  and directory counts

#### Scenario: Cleanup after partial failure

- **GIVEN** Run finished with at least one failed or skipped output
- **WHEN** the summary page advances to cleanup
- **THEN** cleanup confirm is offered with a warning about incomplete export
