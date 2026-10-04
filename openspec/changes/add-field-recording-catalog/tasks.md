# Tasks

## 1. Export tags for FLAC catalog files

- [x] 1.1 Define stable Vorbis key mapping for known `source.bwf` /
      `source.ixml` leaves (cover `CANONICAL_KEYS` BWF/iXML-related fields)
      in `field-audio-io` and document the table in a short code comment or
      SPEC-metadata note; verify with a unit test that Description /
      Originator / a sample iXML leaf map to expected keys
- [x] 1.2 Ensure FLAC encode-with-tags writes the mapped Vorbis comments
      (and omits empty values); verify with an encode round-trip test in
      `field-audio-io`
- [x] 1.3 Wire `user.artist` / `user.copyright` into ARTIST / COPYRIGHT when
      building the Catalog/export TagMap; verify with a unit test that
      non-empty values appear and empty values are omitted
- [x] 1.4 Run `cargo test -p field-audio-io` and fix warnings; verify clean
      focused crate test pass

## 2. Offline multi-output composition render

- [x] 2.1 Introduce a render-plan type (shared DSP slot optional/empty,
      N outputs with path, profile/encode settings, optional chain + params)
      without adding a `field-composition` → `field-audio-monitor` edge;
      verify it compiles in `field-composition`
- [x] 2.2 Implement amortized composition read → per-output identity encode
      path (no DSP) reusing export/tag building; verify a test exports one
      composition to a temp FLAC
- [x] 2.3 Add an offline DSP trait/hook consumed by render (implemented in
      app/scripting host via Faust monitor chains); verify a host-level or
      integration test applies `foa` / `ms` offline to fixture audio and
      produces stereo channel count
- [x] 2.4 Support optional DSP params overriding fixed defaults on a
      per-output basis; verify a test that a supplied param differs from
      default path (or documents default table + override plumbing)
- [x] 2.5 Re-check destination existence immediately before each output
      write; on conflict skip and return structured per-output errors;
      verify with a test that a pre-created dest is skipped
- [x] 2.6 Run `cargo test -p field-composition` (and monitor/host crate
      tests touched); verify focused tests pass with no warnings

## 3. Lua / scripting surface

- [x] 3.1 Expose `composition:render` / `begin_render` (job poll like
      `begin_transcode`) in `field-scripting`; verify a Lua or Rust binding
      test builds a one-output plan successfully
- [x] 3.2 Document the render API and Catalog variables
      (`catalog.library_root`, `catalog.export_profile`) in
      `docs/SCRIPTING.md`; verify the documented names match the
      implementation
- [x] 3.3 FieldAssist `app:confirm` opens a native alert dialog (queued /
      headless default true); Catalog cleanup uses `on_confirm` /
      danger OK; verify unit default/queue plus UI smoke of the dialog
- [x] 3.4 Run `cargo test -p field-scripting` and verify clean pass

## 4. Catalog workflow example

- [x] 4.1 Implement Catalog sheet pages (configure library + profile
      pulldown, run progress, summary, cleanup confirm) in
      `docs/examples/workflows/field-recording/workflow_catalog.lua`
      using keep-group sources; verify script loads (Lua syntax / include
      shared helpers)
- [x] 4.2 Implement destination planning: mirror event dirs, composition
      name basename, layout-code → `st` sidecar naming for
      `ambix`/`fuma`/`ms`, identity-only for mono/stereo/2OA; verify with
      pure Lua unit-style assertions in shared helpers or documented
      examples covering each layout code
- [x] 4.3 Implement preflight (exists + in-plan collisions fail before any
      render) and Run via `begin_render`; verify conflict cases abort before
      encode (manual or scripted checklist against the sheet logic)
- [x] 4.4 Implement cleanup: on confirm, delete session-referenced paths
      under staging only (keep/drop/never-exported); verify outside-staging
      paths are skipped in the helper logic
- [x] 4.5 Update `docs/examples/workflows/field-recording/README.md` and
      note Catalog behavior vs `SPEC-field-recording.md`; verify README
      matches the example entry points

## 5. Integration gates

- [x] 5.1 Run `cargo fmt` and `cargo build` for the workspace; verify no
      errors or warnings
- [x] 5.2 Run `cargo test` for the workspace; verify all tests pass
- [ ] 5.3 Manual smoke: Review keep FOA + stereo takes → Catalog into a
      temp library → confirm files, tags, sidecar names, then cleanup;
      verify staging refs removed and Sononym-ready FLAC layout on disk
