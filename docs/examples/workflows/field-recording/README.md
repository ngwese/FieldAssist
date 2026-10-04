# Field-recording pipeline examples

Copyable Lua for the **Ingest → Review → Catalog** reference pipeline specified
in [SPEC-field-recording.md](../../spec/SPEC-field-recording.md).

These scripts are **not** embedded in the FieldAssist binary. The Review example
matches the built-in's Keep / Drop behavior and runs on today's host; copying it
into the config directory **overrides** the embedded `review` registration.

## Files

| File | Role |
| --- | --- |
| `shared.lua` | Property helpers, media expand, group filter, and `field.url` / `field.fs` comments |
| `workflow_ingest.lua` | Stateful Ingest: backup, convert to staging, verify, optional source delete |
| `workflow_review.lua` | Stateful Review: Keep / Drop |
| `workflow_catalog.lua` | Stateful Catalog: preflight, multi-output render (`composition:begin_render`), summary, staging cleanup confirm |

Workflows load helpers with `field.include("shared.lua")` and use the `field.*`
namespace (`field.workflow`, `field.session.focused()`, `field.ui`, …).

## How to try them

1. Copy `shared.lua` and the three `workflow_*.lua` files into the FieldAssist
   config directory (same place as `init.lua`). On macOS that is
   `~/Library/Application Support/FieldAssist/`.
2. Restart FieldAssist (or reload scripts if a future host supports that).
3. Ingest: drop a recorder folder on the Ingest overlay, or start Ingest from
   the Workflow menu and set Source / Backup / Staging paths. Finish hands off
   to Review (menu prompt until `finish_workflow({ next = … })` ships).
4. Review: Keep / Drop pass. Finish prompts to start Catalog.
5. Catalog: Configure → Run → Summary → Cleanup (Ingest-shaped sheet).
   - Configure: Library defaults to `${catalog.library_root}` with a muted
     resolved preview, selection size for kept compositions, and Profile.
   - Run: deferred `begin_render` polling with composition + render progress
     bars and an Issues log (errors/warnings). Warnings/errors keep you on
     Run; Back returns to Configure.
   - Summary: rendered file count, total size, destination; **Finish**
     (outline) or **Cleanup**.
   - Cleanup: scrollable list of staging paths (plus empty parent dirs that
     would remain); **Back** (to Summary), **Cancel** (exit workflow), or
     alert-colored **Cleanup**. Paths outside staging are never touched;
     empty parents under staging are removed after their files, and the
     summary notes `N file(s) under staging and M director(y/ies)` when
     applicable.

## Catalog behavior vs SPEC-field-recording.md

| SPEC requirement | Implementation |
| --- | --- |
| Source set = `group == "keep"` | `shared.docs_in_group(session, "keep")` |
| Library root from `catalog.library_root` / `user.catalog.library_root` | `${catalog.library_root}` field + resolve fallback |
| Export profile from `catalog.export_profile` or pulldown | `Catalog.profile` + sheet `select` control |
| Mirror event dirs: `{library}/{event}/{name}.{ext}` | `Catalog:event_dir()` (`source.event.name`, else media/comp parent basename) |
| Copy `.facomp` / `.fasession` beside renders | `plan_project_copies` + copy after each comp / end of run |
| Stereo sidecar for `ambix`/`fuma`/`ms` layout codes | `Catalog:mixdown_chain()` + `sidecar_basename()` |
| Identity-only for mono / stereo / 2OA | No `chain` key on the identity output |
| Preflight: exists + in-plan collision check before Run | `Catalog:preflight()` — failures go to Issues log |
| Re-check dest before each write (mid-run conflict) | `render_outputs_from_planes` in `field-composition` |
| Summary: file count / size / destination | Summary sheet pane |
| Cleanup list + confirm for staging files only | Resolves relative media URLs; includes `.fasession`; closes docs then deletes |

Notes:
- The Faust offline DSP for stereo mixdown requires FieldAssist's `DesktopBackend`
  (`create_offline_dsp`). In headless / `field-batch`, `chain` outputs are
  skipped (DSP returns `nil`), producing identity-only output.
- Cleanup uses FieldAssist’s native `app:confirm` dialog (`ok = "Cleanup"`,
  `danger = true`, work in `on_confirm`).
- `finish_workflow({ next = … })` handoff is stubbed (not yet shipped).

## Shared session properties

See the spec. In brief: `ingest_source`, `backup_root`, `staging_root`,
`catalog_library_root`. The Catalog workflow also reads
`user.catalog.library_root` from user variables as a fallback.
