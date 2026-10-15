# Tasks

## 1. Extract guide and reference files

- [x] 1.1 Create `docs/scripting/guide.md` from `docs/SCRIPTING.md`
      lines 101-244 (Hosts and scope, Initialization, Conventions) under
      a single H1; verify it reads as one document and heading levels
      are consistent
- [x] 1.2 Create `docs/scripting/reference/app.md` from lines 245-302;
      verify it has exactly one H1 and the host-only `app` material
- [x] 1.3 Create `docs/scripting/reference/field-log.md` (303-323),
      `field-on.md` (324-407), `field-include.md` (408-431),
      `field-scripting.md` (432-475); verify each has one H1
- [x] 1.4 Create `docs/scripting/reference/field-url.md` (476-522),
      `field-fs.md` (523-547), `field-session.md` (548-605),
      `field-composition.md` (606-756); verify each has one H1
- [x] 1.5 Create `docs/scripting/reference/field-media.md` (757-844),
      `field-layouts.md` (845-901), `field-exports.md` (902-1027),
      `field-variables.md` (1028-1151); verify each has one H1
- [x] 1.6 Create `docs/scripting/reference/field-workflow.md`
      (1152-1239), `field-ui.md` (1240-1315),
      `field-audio_devices.md` (1316-1346), `nested-types.md`
      (1347-1383); verify each has one H1
- [x] 1.7 Create `docs/scripting/README.md` as the hub: intro text from
      lines 1-15 plus a short list linking guide and each reference page;
      drop the 89-line table of contents
- [x] 1.8 Delete `docs/SCRIPTING.md`; verify no file under `docs/` or
      `README.md` links to it (link pass is task 3)

## 2. Build the anchor rewrite table and rewrite links

- [x] 2.1 Enumerate the unique `](#...)` anchors remaining after the
      preamble and table of contents were separated, and map each to its
      target page; the split leaves 18 unique anchors, all module or
      returned-type references
- [x] 2.2 Rewrite module and returned-type anchors in the reference
      pages to relative file links; verify no `](#field` remains
- [x] 2.3 Confirm guide anchors resolve within `guide.md` (the guide
      keeps Hosts and scope, Initialization, and Conventions as its own
      sections) and that section cross-links point at `reference/` pages
      where they leave the guide; verify both resolve
- [x] 2.4 Fix the external links (`../spec/SPEC-metadata.md`,
      `../../examples/field-batch/lsqlite-module/`) for their new depth
      under `docs/scripting/reference/`; verify each resolves
- [x] 2.5 Re-read `docs/scripting/` prose for self-references that
      assumed one document; verify no dangling self-references remain

## 3. Repoint incoming references

- [x] 3.1 Update `README.md` (2 links) to point at
      `docs/scripting/README.md`; verify links resolve
- [x] 3.2 Update `docs/ARCHITECTURE.md` (1 link); verify it resolves
- [x] 3.3 Update `docs/spec/SPEC-application.md` (3),
      `SPEC-analysis.md` (2), `SPEC-metadata.md` (2),
      `SPEC-workflows.md` (2), `SPEC-field-recording.md` (2); verify
      each resolves
- [x] 3.4 Update `docs/examples/workflows/field-recording/workflow_catalog.lua`
      (comment) and
      `docs/examples/field-batch/lsqlite-module/README.md` (1 link);
      verify each resolves
- [x] 3.5 Update `openspec/config.yaml` (2 mentions of
      `docs/SCRIPTING.md`) to the new locations; verify the text reads
      correctly
- [x] 3.6 Leave archived change artifacts under
      `openspec/changes/archive/` unchanged; verify none were edited

## 4. Add the documentation check

- [x] 4.1 Write `script/check-scripting-docs` (shell or small script)
      that verifies: no `](#field` / `](#app` anchors remain; every
      relative link under `docs/scripting/` resolves; each reference
      page has exactly one H1; the set of module pages matches the
      documented `field.*` / `app` modules
- [x] 4.2 Run the check against the finished tree; verify it passes
- [x] 4.3 Confirm the check fails on a deliberately broken link, on a
      reintroduced `](#field.media)` anchor, and on a second H1, then
      revert the probes; verify all three failures are caught

## 5. Gates

- [x] 5.1 Run the documentation check; verify it passes clean
- [x] 5.2 Confirm the repository has no markdown lint configuration to
      run over `docs/scripting/`; carry the former `SCRIPTING.md`
      `markdownlint-disable MD013 MD060` annotation onto each split page
      so long table lines stay exempt, and record that no lint gate
      exists to run
- [x] 5.3 Confirm no Rust source, `Cargo.toml`, or workflow file was
      changed; verify the diff is documentation, script, and
      `openspec/config.yaml` only. Run `cargo fmt --check`, `cargo
      build`, and `cargo test` to confirm the workspace is unaffected
