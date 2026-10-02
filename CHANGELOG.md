# CHANGELOG

<!-- git-cliff: end of header -->

## [0.22.0] - 2026-10-02

### Features

- Expose ingest staging/backup paths with resolved previews
- Rename ingest Stage pane to Run and auto-finish
- Tighten workflow sheet chrome and match export path browse
- Show ingest path capacity via field.fs.available_space

### Bug Fixes

- Read env variables via Rust for Windows
- Keep window movable while workflow sheet is open
- Stop analysis progress from restarting mid-pass
- Replace flacenc with flac-codec for FLAC encode


## [0.21.0] - 2026-09-30

### Features

- Add user sub-scopes, env vars, and recursive interpolate
- Add site VariableResolver with resolve and expand
- Load variables.json in the shared scripting host
- Add streaming bandlimited StreamingResampler
- Add streaming EncodeStream and file transcode
- Add field.media.open, pool intern, and transcode
- Add export registry find and profile extension
- Expand VariableResolver resolve values on demand
- Add workflow toolbar select and path browse prefix
- Stage ingest media via field.media.transcode
- Add modal workflow sheets for batch ingest
- Poll background transcode progress in ingest sheet

### Bug Fixes

- Scope release changelogs to commits since prior tag
- Read env variables via Rust so Windows matches templates

### Documentation

- Document media open/transcode and add staging example

### Miscellaneous Tasks

- Ignore .DS_Store files
- Upgrade gpui-kit to 0.7.0


## [0.20.0] - 2026-09-29

### Features

- Add ${…} variable completion for export paths
- Add scripting search path for resolvers and workflows
- Share settings across hosts and extend package.path


## [0.19.0] - 2026-09-27

### Features

- Add editable spectrum heatmap gradient settings
- Add ChannelToggle chips for channel selection
- Add optional layout code for export filenames
- Add enrich_composition hook for media-built docs
- Add enrich_session hook for new empty sessions

### Bug Fixes

- Use theme primary for active UI indicators
- Shorten mono layout channel label to C
- Format ChannelSelector render chain
- Give export sheet actions a shared rem width
- Dismiss export sheet only via Cancel or Escape

### Refactor

- Extract ChannelSelector for shared channel chips


## [0.18.0] - 2026-09-27

### Features

- Add scoped variables, resolvers, and Variables pane
- Add User Variables window and shared table chrome
- Version settings.json with kind and format_version
- Wire Export sheet variables and path templates

### Bug Fixes

- Give dpkg-shlibdeps a valid Architecture field
- Make soft_resolved_path test OS-agnostic

### Documentation

- Document clearing DMG quarantine with xattr


## [0.17.0] - 2026-09-26

### Features

- Ship an amd64 .deb alongside the Linux tarball
- Add threaded overview peak rendering settings
- Theme Settings and About with custom title bars

### Bug Fixes

- Fold PCM peaks below peak-block zoom
- Align settings-load tests with quiet view defaults

### Miscellaneous Tasks

- Rotate macOS code-signing certificate
- Trim changelog and stabilize git-cliff header
- Quiet view defaults and compact settings UI


## [0.16.0] - 2026-09-26

### Miscellaneous Tasks

- Publish MSIX, DMG, and Linux tarball only
