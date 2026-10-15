# Design

## Context

`docs/SCRIPTING.md` is a single 1383-line, hand-authored Markdown file
with one H1, 21 H2, 63 H3, and 34 H4. It has no frontmatter and no
generator; it is edited by hand and matched to implementation by review.
It combines:

- An intro and a hand-maintained 89-line table of contents (lines 1-99).
- A guide-flavored preamble: Hosts and scope (101-137), Initialization
  (138-229), Conventions (230-244).
- A reference body: `app` (host-only) plus 19 `field.*` modules
  (245-1346), and Nested types (1347-1383).

Cross-references are 109 intra-document `#anchor` links (83 unique),
almost all pointing at reference sections such as `#fieldvariables`.
There are no relative-path links in the reference body. Two external
links (`spec/SPEC-field-recording.md`, `spec/SPEC-metadata.md`) and two
`examples/` links live in the intro and module prose.

A separate `field-docs` repository will render this content as its
scripting section and consume it through a git submodule plus a
sparse-checkout limited to `docs/scripting`, `docs/spec`, and
`docs/examples`. That site needs stable per-module files. A submodule
cannot point at a subdirectory, so the source layout must stay shallow
enough that a sparse path is useful.

## Goals / Non-Goals

**Goals:**

- One file per reference module, at a predictable path.
- One guide document for the preamble material.
- Every internal and incoming link resolves after the split.
- A mechanical check that catches drift from the split.

**Non-Goals:**

- Source-derived documentation generation.
- Content rewrite beyond split mechanics, heading normalization, and link
  repair.
- Site scaffolding; that lives in the `field-docs` repository and its own
  OpenSpec change.
- Any Rust or runtime change.

## Decisions

### 1. Split at the Conventions / `app` boundary

**Choice:** Use the existing structural boundary between the guide
preamble and the reference body: `## Conventions` ends at line 244 and
`## \`app\` (host-only)` begins at line 245. Everything before the
boundary becomes `guide.md`; everything from it becomes `reference/`.

**Rationale:** The document already separates these two voices. Hosts and
scope, Initialization, and Conventions are read once to learn how
scripting runs; module sections are looked up by name.

**Alternatives considered:**

- Split per module but keep the preamble in the repository README —
  rejects the initialization material a guide reader needs in one place.
- Keep one file and let the site fragment it at build time — moves parsing
  into `field-docs`, which then owns fragility for content it does not
  own.

### 2. One file per H2 module section

**Choice:** Each `## \`field.x\`` section becomes
`docs/scripting/reference/<slug>.md`, with the module name's backticks and
dots removed and dots replaced by `-` (`field.composition` ->
`field-composition.md`). `app` (host-only) becomes `app.md`; Nested types
becomes `nested-types.md`. The three preamble sections move into
`docs/scripting/guide.md`.

**Rationale:** A module is the unit a reader looks up and an
implementation change touches. One file per module yields one site
sidebar entry and a stable URL per module.

**Alternatives considered:**

- Split at H3 — produces dozens of tiny pages and separates "Returned
  type" blocks from their module.
- Group into a few large pages — keeps the original navigation problem at
  a smaller scale.

### 3. Heading normalization

**Choice:** Each reference file's top H2 becomes its H1; nested headings
shift only where needed to keep one H1 per file and a sensible outline.
`guide.md` keeps multiple H2 sections under one H1.

**Rationale:** The site renders one page per file, so a page starting at
H2 renders an orphaned heading. One H1 per file also satisfies
markdownlint and gives each page a title.

**Alternatives considered:**

- Leave H2 and let the theme demote — fragile across renderers and leaves
  no page title.

### 4. Intra-document anchors become relative file links

**Choice:** Rewrite each `](#anchor)` as a relative link. An anchor that
resolves to a module or a "Returned type" / "Properties" / "Functions"
sub-heading becomes a link to that module's page (`#field.variables` ->
`field-variables.md`; `#returned-type-composition` ->
`field-composition.md`). Guide anchors (`#hosts-and-scope`,
`#initialization`, `#conventions`, `#fieldassist`, `#field-batch`,
`#field-play`) become `../guide.md` links with a fragment. The
anchor-to-target mapping is built and reviewed before rewriting.

**Rationale:** Relative links resolve both in the GitHub repository view
and in the rendered site, and do not depend on the site's URL shape or
base path.

**Alternatives considered:**

- Absolute site paths (`/reference/scripting/field-variables/`) — break
  in the repository and hard-code the site layout, including its base.

### 5. Drop the hand-maintained table of contents

**Choice:** Remove lines 16-99. `docs/scripting/README.md` carries a short
hub list linking the guide and the module pages in a stable order.

**Rationale:** A hand-maintained 89-line TOC is exactly what drifts after
a split. The site derives navigation from the file tree; the repository
hub needs only a short list.

**Alternatives considered:**

- Regenerate the TOC mechanically — adds a generator to a hand-authored
  document; the hub list is simpler and sufficient.

### 6. A documentation check replaces the generator's guarantee

**Choice:** Add `script/check-scripting-docs`, which fails when:

- a `](#field...)` or `](#app...)` anchor remains under `docs/scripting/`;
- a relative link under `docs/scripting/` does not resolve to a file;
- a reference page has zero or more than one H1;
- the set of `field.*` / `app` modules documented does not match the set
  of reference pages.

**Rationale:** Without a generator, the split's main risk is silent link
rot. A small mechanical check covers the properties that matter and runs
in the repository, not only in the site build.

**Alternatives considered:**

- A full generator — out of scope (see proposal non-goals).
- No check — accepts the specific silent-drift risk this change
  introduces.

### 7. Layout stays submodule-ready

**Choice:** Keep the per-file structure shallow and stable, and add no
generated index or manifest under `docs/scripting/`. Ordering and
sectioning for the site are the site's concern.

**Rationale:** The `field-docs` change is a separate repository and a
separate OpenSpec change. This change must not create generated content
that `field-docs` would have to own or regenerate.

**Alternatives considered:**

- Emit a manifest such as `_meta.json` for a site sidebar — moves site
  concerns into the source repository; deferred unless the site needs it.

## Open Questions

- Whether the `field-docs` site needs an explicit ordering or sectioning
  manifest. Deferred to that repository's change; the reference page
  order here follows the document's current order.
