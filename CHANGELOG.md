# CHANGELOG

<!-- git-cliff: end of header -->

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
