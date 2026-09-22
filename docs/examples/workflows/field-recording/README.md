# Field-recording pipeline examples

Copyable Lua for the **Ingest → Review → Catalog** reference pipeline specified
in [SPEC-field-recording.md](../../spec/SPEC-field-recording.md).

These scripts are **not** embedded in the FieldAssist binary. Ingest and Catalog
call intended host APIs that do not ship yet. The Review example matches the
built-in’s Keep / Drop behavior and runs on today’s host; copying it into the
config directory **overrides** the embedded `review` registration.

## Files

| File | Role |
| --- | --- |
| `shared.lua` | Property helpers, media expand, group filter, and comments for intended `field.url` / `field.fs` / sqlite |
| `workflow_ingest.lua` | Stateful Ingest: backup, convert to staging, verify, optional source delete |
| `workflow_review.lua` | Stateful Review: Keep / Drop (no Output field; library path is Catalog’s) |
| `workflow_catalog.lua` | Stateful Catalog: export `keep` documents, then optional staging cleanup |

Workflows load helpers with `field.include("shared.lua")` and use the `field.*`
namespace (`field.workflow`, `field.session.shared()`, `field.ui`, …).

## How to try them

1. Copy `shared.lua` and the three `workflow_*.lua` files into the FieldAssist
   config directory (same place as `init.lua`). On macOS that is
   `~/Library/Application Support/FieldAssist/`.
2. Restart FieldAssist (or reload scripts if a future host supports that).
3. Ingest: drop a recorder folder on the Ingest overlay, or start Ingest from
   the Workflow menu and set Source / Backup / Staging paths. Finish hands off
   to Review (menu prompt until `finish_workflow({ next = … })` ships).
4. Review: Keep / Drop pass. Finish prompts to start Catalog.
5. Catalog: set Library (`catalog_root`), export kept takes; optional staging
   cleanup once confirm / fs APIs exist.

Until the host APIs land, Ingest and Catalog may error at first use of an
undefined method. That is expected for those stages.

## Shared session properties

See the spec. In brief: `ingest_source`, `backup_root`, `staging_root`, and
`catalog_root`. (The shipping built-in Review still uses `output`; the pipeline
example does not.)
