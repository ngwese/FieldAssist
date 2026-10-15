# Scripting

As-built Lua 5.4 reference for FieldAssist, **field-batch**, and
**field-play**. Hosts share the `field-scripting` API shape under the
global `field` namespace. The global `app` object is a thin host facade;
`app.name` identifies the process.

This documentation describes **what is implemented today**. Intended-but-missing
APIs (confirm dialogs, SQLite, C2PA, handoff) live in
[spec/SPEC-field-recording.md](../spec/SPEC-field-recording.md). Copyable
pipeline examples are under
[examples/workflows/field-recording/](../examples/workflows/field-recording/).

Start with the [scripting guide](guide.md) for how the hosts initialize and
the conventions the API follows, then look up a module in the reference.

## Guide

- [Scripting guide](guide.md) — host scope, initialization, conventions

## Reference

- [`app` (host-only)](reference/app.md)
- [`field.log`](reference/field-log.md)
- [`field.on`](reference/field-on.md)
- [`field.include`](reference/field-include.md)
- [`field.scripting`](reference/field-scripting.md)
- [`field.url`](reference/field-url.md)
- [`field.fs`](reference/field-fs.md)
- [`field.session`](reference/field-session.md)
- [`field.composition`](reference/field-composition.md)
- [`field.media`](reference/field-media.md)
- [`field.layouts`](reference/field-layouts.md)
- [`field.exports`](reference/field-exports.md)
- [`field.variables`](reference/field-variables.md)
- [`field.workflow`](reference/field-workflow.md)
- [`field.ui`](reference/field-ui.md)
- [`field.audio_devices`](reference/field-audio_devices.md)
- [Nested types](reference/nested-types.md)
