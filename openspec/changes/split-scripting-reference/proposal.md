# Proposal

## Why

The Lua 5.4 scripting surface shared by FieldAssist, `field-batch`, and
`field-play` is documented in a single 1383-line `docs/SCRIPTING.md`. It
mixes two voices in one file: a short guide preamble (host scope,
initialization, conventions) and a large as-built reference for `app`
plus 20 `field.*` modules. A flat file that size is hard to navigate and
cannot be published as a documentation site's reference section without
being split first. A separate `field-docs` site will consume this content
through a git submodule.

## What Changes

- Delete `docs/SCRIPTING.md` and split its content at its natural seam
  (the `## Conventions` / `## \`app\`` boundary) into
  `docs/scripting/guide.md` (Hosts and scope, Initialization, Conventions)
  and `docs/scripting/reference/*.md` (one page per `app` / `field.*`
  module, plus a Nested types page).
- Normalize heading levels so each reference page has exactly one H1, and
  drop the hand-maintained 89-line table of contents; a short hub in
  `docs/scripting/README.md` replaces it.
- Rewrite the intra-document `#anchor` cross-references (109 links, 83
  unique) as relative file links that resolve within `docs/scripting/`.
- Repoint every incoming reference to `SCRIPTING.md` in tracked,
  non-archived files: `README.md`, `docs/ARCHITECTURE.md`, five
  `docs/spec/SPEC-*.md` files, two `docs/examples/` references, and
  `openspec/config.yaml`.
- Add a repository check that fails on a broken relative link, a
  remaining `#field`/`#app` anchor, a reference page without exactly one
  H1, or a mismatch between documented modules and reference pages.

No runtime behavior changes. No Rust source, crate, or workflow changes.

## Capabilities

### New Capabilities

- (none — this change alters documentation structure, not product
  behavior)

### Modified Capabilities

- (none — no spec-level behavior changes; the change is docs-only and
  sets `skip_specs: true`)

## Impact

- **Hosts:** none. FieldAssist, `field-batch`, and `field-play` are
  unchanged.
- **Crates:** none.
- **Realtime risk:** none. No playback or monitor code is touched.
- **Docs:** `docs/SCRIPTING.md` removed; `docs/scripting/README.md`,
  `docs/scripting/guide.md`, and 17 reference pages added; inbound links
  updated in `README.md`, `docs/ARCHITECTURE.md`, `docs/spec/`, and
  `docs/examples/`; `openspec/config.yaml` updated; a new
  `script/check-scripting-docs` check added.
- **External:** the sibling `field-docs` repository consumes
  `docs/scripting/` through a submodule and sparse-checkout. This change
  keeps that layout shallow and stable and adds no generated index that
  `field-docs` would have to regenerate.
- **Gates:** documentation-only, so `cargo fmt` / `build` / `test` do not
  apply. Run the new docs check and the existing markdown lint clean.
