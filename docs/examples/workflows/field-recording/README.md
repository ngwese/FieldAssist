# Field-recording pipeline examples

Copyable Lua for the **Ingest → Review → Catalog** reference pipeline specified
in [SPEC-field-recording.md](../../spec/SPEC-field-recording.md).

These scripts are **not** embedded in the FieldAssist binary and are **not**
expected to run until the intended host APIs ship (`app:confirm`, `app.url`,
`app.fs`, `app.sqlite`, metadata/encode, background jobs, and
`app:finish_workflow({ next = … })`).

## Files

| File | Role |
| --- | --- |
| `shared.lua` | Property helpers and comments for the intended `app.url` / `app.fs` / `app.sqlite` surface |
| `workflow_ingest.lua` | Stateful Ingest: backup, convert to staging, verify, optional source delete |
| `workflow_catalog.lua` | Stateful Catalog: export `keep` documents, then optional staging cleanup |

**Review** is the built-in workflow already shipped with the app. Do not copy a
fork of `workflow_review.lua`; after Ingest, start **Review** from the Workflow
menu (or via the intended handoff).

## How to try them (once APIs exist)

1. Copy `shared.lua`, `workflow_ingest.lua`, and `workflow_catalog.lua` into
   the FieldAssist config directory (same place as `init.lua` /
   `workflow_*.lua`). On macOS that is
   `~/Library/Application Support/FieldAssist/`.
2. Restart FieldAssist (or reload scripts if a future host supports that).
3. Ingest: drop a recorder folder on the Ingest overlay, or start Ingest from
   the Workflow menu and set Source / Backup / Staging paths.
4. Review: built-in Keep / Drop pass on staged documents; set Output to your
   library folder if desired.
5. Catalog: start Catalog from the menu; it defaults `catalog_root` from
   Review’s `output`.

Until the host APIs land, declaring these workflows may error at first use of
an undefined method. That is expected for a spec example.

## Shared session properties

See the spec. In brief: `ingest_source`, `backup_root`, `staging_root`,
`catalog_root`, and Review’s `output`.
