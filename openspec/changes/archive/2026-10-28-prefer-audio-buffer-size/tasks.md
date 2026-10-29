# Tasks

## 1. Settings key rename

- [x] 1.1 Rename `AudioSettings.period_frames` to `buffer_size_frames` with
      `serde(alias = "period_frames")`, update `opened_*` helper to stop
      auto-persisting on open (or remove it), and adjust
      `field-settings` unit tests so legacy JSON loads and new saves emit
      `buffer_size_frames` only — verify with
      `cargo test -p field-settings`
- [x] 1.2 Update FieldAssist / field-play call sites that read
      `audio.period_frames` to `buffer_size_frames` and remove
      persist-on-open wiring in `app.rs` — verify the workspace compiles
      for those crates (`cargo build -p field-assist -p field-play`)

## 2. Playback negotiation

- [x] 2.1 Rename public period types/APIs toward preferred buffer size
      (`PreferPlatformDefault`, `OpenedPeriod` + `actual_frames`, catalog
      helpers as needed) and update exports/docs in
      `field-audio-playback` — verify `cargo build -p field-audio-playback`
- [x] 2.2 Implement range-aware candidate ordering (prefer ≥2× probed
      default within `SupportedBufferSize`, then larger, then smaller,
      then Default); try user `Frames(n)` first then the same path —
      verify new unit tests for candidate selection cover in-range,
      clamped, and unknown-range cases (`cargo test -p field-audio-playback`)
- [x] 2.3 After successful open, read `stream.buffer_size()`, populate
      `actual_frames`, and log the actual size (Fixed and Default) —
      verify logging path via unit/integration coverage or a focused
      open test where the host allows, and
      `cargo test -p field-audio-playback`
- [x] 2.4 Update `crates/field-audio-playback/AGENTS.md` host/period
      section to describe preferred buffer negotiation — verify the doc
      matches the new public API names

## 3. Hosts and UI

- [x] 3.1 Settings UI: rename "Period" → "Buffer Size", update description
      to preferred/not guaranteed, bind to `buffer_size_frames` — verify
      reset clears preference and triggers reopen without writing a
      negotiated value
- [x] 3.2 field-play: use renamed settings field and print/log actual
      opened buffer size — verify `cargo build -p field-play` and help/UI
      strings no longer say "period" where they mean buffer size

## 4. Product docs

- [x] 4.1 Update `docs/ARCHITECTURE.md`, `docs/spec/SPEC-application.md`,
      and `docs/scripting/guide.md` for `buffer_size_frames` and
      negotiate-on-open / no auto-persist — verify docs no longer
      reference `period_frames` or snap-and-store behavior

## 5. Verification gates

- [x] 5.1 Run `cargo fmt`, then `cargo build` and `cargo test` with no
      errors and no warnings across the workspace
