## [0.15.0] - 2026-09-26

### Features

- Ship Windows builds as a per-user MSIX
- Read MSIX PFX password from 1Password
- Ship macOS DMG with bundled CLIs and Install CLI Tools
- Ship a combined Linux release tarball

### Documentation

- Document MSIX conflict cleanup and op password for packs

### Miscellaneous Tasks

- Prepare release 0.12.0
- Bump workspace version to 0.13.0-pre
- Bump workspace version to 0.15.0-pre
## [0.12.0] - 2026-09-25

### Features

- Add scrollable multi-channel waveform viewer
- Add application menu with About and Quit
- Show in-window app menu on Windows and Linux
- Add waveform selection model and annotation UX
- Add cpal playback with transport and playhead sync
- Add optional launch, open dialog, and drag-drop
- Add macOS app bundle and CLI install
- Add JSON keymap and view frame commands
- Add View menu and hover ghost playhead
- Add Windows GUI exe and CLI install script
- Add centered Lucide icon transport bar
- Capture bit depth from loaded audio files
- Add status bar with loaded file metadata
- Add clip composition model and Edit menu
- Show generic progress while building waveform peaks
- Dock the waveform beside a collapsible Edits pane
- Highlight edit ranges and restore original media on Init
- Scroll to an edit on click, jump on double-click
- Adopt a custom TitleBar with an embedded menu
- Open multiple compositions in session tabs
- Add Lua scripting with a Script dock
- Save and open .facomp projects and render audio
- Add composition markers and speed up project open
- Add FieldAssist mark as the application icon
- Track composition dirtiness from the clean baseline
- Prompt to save unsaved compositions on close and quit
- Rebrand as FieldAssist 0.2.0
- Rename History dock to Detail
- Persist named collections and marker type definitions
- Add scriptable channel layouts and muted chrome
- Add Lua logging API and Messages tab
- Add Faust monitor chains and a Monitor tab
- Polish Monitor controls and status-bar speaker toggle
- Add B-format AmbiX and FuMa monitor layouts
- Pin monitor params and polish composition tabs
- Seek transport without stopping playback
- Add a session output device picker
- Persist Session as .fasession and expose it to Lua
- Rebuild file drag-drop as Lua workflows
- Add stateful Lua workflows with a session toolbar
- Add a Workflow menu after View
- Group compositions in the explorer
- Add workflow toolbar widgets and pane show/hide commands
- Associate .facomp and .fasession on macOS
- Advance to next todo when marking reviewed
- Use circle-play for status bar preview
- Show git revision in About and add field-assist CLI
- Add FieldAssist app menu and File menu
- Reorder session documents via explorer and Lua
- Rebuild Lua workflows on prototype/instance pattern
- Restyle markers and regions list panels
- Rework monitor panel with RMS meters and Output/Gain
- Run monitor DSP on the realtime callback
- Break out selection into child compositions
- Improve compositions explorer ergonomics
- Surface playback xruns in the Messages panel
- Improve composition break-out naming and explorer cues
- Show message counts on the session status bar
- Clear status-bar message counts on Shift-click
- Add initial pull-based audio analysis
- Add Spectrum view with progressive analysis streams
- Port MinMaxOp and fix spectrum region overlays
- Add Peaks + Spectrum shared lane view
- Regional analysis recompute after edits
- Tint monitor status icon red when output is faulted
- Report output open failures in Messages and status bar
- Open Finder and Dock documents via macOS Apple Events
- Share analysis I/O across peaks and spectrum
- Add macOS Hide, Hide Others, and Show All
- Improve Messages table and composition list display
- Address media by content hash in shared session stores
- Play media files through field-play
- Add shared media pool Lua API and Media dock
- Add View Follow Playhead toggle
- Add File Close Session
- Expand directories in the Add drag-drop workflow
- Move Zero Crossing into the Selection menu
- Break out compositions from regions or channels
- Add waveform peak and spectrum scale gutters
- Add app.ui toolbar controls and Review Keep chrome
- Add Lua theme selection and vendor GPUI themes
- Add Script and Media toggles to the status bar
- Extract field-scripting and field-batch Lua hosts
- Enrich audio device listing and install CLI tools
- Show version and git revision in field-batch REPL
- Lock down Lua require paths with opt-in system modules
- Migrate FieldAssist onto shared field-scripting host
- Drive field-play monitor chain via detect_layout
- Load shared embedded init.lua in field-batch
- Add interactive transport UI to field-play
- Add markers, notes, and save to field-play
- Toggle looping with l in field-play
- Support native Lua C modules for field-batch
- Install field-play and field-batch on Windows
- Expand Lua layout registry and media APIs
- Aggregate open failures into a load-problems dialog
- Add waveform quick note on n
- Add Settings window backed by settings.json
- Add experimental feature flags and gate Analyze
- Rename Render to Export and add Lua export profiles

### Bug Fixes

- Enable File Open from the macOS menu bar
- Restart playback from the start at buffer end
- Keep playhead visible at last sample
- Keep playback stable while hovering edit cards
- Keep the UI responsive while opening large files
- Hide the title-bar app icon on macOS
- Keep playhead and peaks aligned after inserts
- Keep the compositions list closed at startup
- Keep the UI responsive after edits and History
- Scope Space play/pause to the waveform
- Zoom the scroll wheel around the pointer
- Make the loop toggle fill stronger than hover
- Inset waveform channel labels from the header
- Ignore layout and monitor when marking dirty
- Default save name to basename.facomp
- Default session save name from workflow id
- Avoid nested WorkflowBar update on path edit
- Stabilize workflow toolbar path field editing
- Inset macOS status bar from rounded window corners
- Log Lua timings with two decimal places
- Clamp side docks to tab-bar minimum while dragging
- Follow Windows and Linux menu conventions
- Keep decode and pager I/O off the audio callback
- Match REPL input style to history transcript
- Make explorer end-of-list drop reorder reliably
- Clear unused warnings and prefetch monitor tests
- Stop Faust from thrashing monitor rebuilds
- Paint session waveforms as peak blocks land
- Wrap loop playhead and bind playback after session open
- Seek to region start when selecting during play
- Cut idle CPU when monitor pane is open
- Keep Space play/pause after waveform hover
- Tint VU meters with theme chart_3 blue
- Use 5ms attack on Envelope Peak follower
- Clip spectrum tiles on horizontal shrink
- Use radio dots for macOS waveform View items
- Use bandlimited SRC on playback prefetch
- Keep waveform representation app-global
- Keep macOS menus working with no editor window
- Timeout CPAL output open and allow disabled engine
- Fall back to disabled playback on open timeout
- Omit custom title bar in fullscreen
- Use platform-native quit menu labels
- Keep session composition ids aligned with files
- Stabilize Follow Playhead scroll and peaks paint
- Keep waveform hover under the pointer while scrolling
- Hide composition reorder marker during dock resize
- Make Clear consistent and idempotent in EDL history
- Keep selection when toggling Reviewed
- Limit dB/Hz hover to the waveform canvas
- Avoid RefCell panic when Lua sets active composition
- Print field-batch REPL banner before init logs
- Install block decoder when headless hosts open media
- Simplify field-play unsaved quit prompt to y/n
- Resolve Lua media fields in the live desktop backend
- Surface Lua runtime errors instead of masking them
- Make FLAC renders readable in FieldAssist
- Clear monitor chain on channel break-out
- *(ci)* Install ALSA on Linux and skip play test without device
- *(ci)* Import cpal HostTrait in field-play CLI test
- *(ci)* Harden headless audio skip and Windows scripting tests
- Make git-cliff prepend work without section emojis

### Documentation

- Add MIT license, README, and SPDX headers
- Clarify 80-column wrapping for commit messages
- Add TODOs for render, facomp tests, and reload
- Add as-built product specifications
- Move BUILDING.md into docs/
- Tidy README doc links and crate table
- Add app screenshot to README
- Warn that FieldAssist is under development
- Note supported platforms in README
- Fix README warning admonition formatting
- Note Ubuntu packages required to build
- Require cargo fmt before committing code
- Add draft analysis product specification
- Document app.ui toolbar constructors and icons
- Update workflow spec for Keep groups and toolbar API
- Note Review Keep groups in the application spec
- Specify Ingest → Review → Catalog multi-step workflows
- Add structured Review example to field-recording pipeline
- Document field-batch and stop calling field-play an example
- List supported codecs in CLI help
- List decode formats and codecs in README and CLI
- Clarify version-tags ruleset behavior

### Performance

- Keep waveform paint and playback off the pager hot path

### Refactor

- Extract WaveformDisplay with WaveformDataProvider
- Move About to Help menu
- Split FieldAssist into a Cargo workspace
- Genericize field-ui-components behind host traits

### Styling

- Rustfmt play icon asset include
- Mute header and transport icon buttons
- Wrap previous_anchor_near on one line
- Color Messages info logs green
- Shorten About label and bold app menu title
- Rustfmt import order and wrapping

### Miscellaneous Tasks

- Upgrade symphonia to 0.6
- Upgrade cpal to 0.18
- Upgrade mlua to 0.12
- Upgrade ogg to 0.9
- Add SPDX headers to monitor DSP files
- Bump version to 0.4.0
- Note grouping monitor chain controls
- Note MSIX packaging and file-type associations
- Finish renaming snd-review to FieldAssist
- Note workflow toolbar command/callback cleanup
- Bump version to 0.6.0
- Bump version to 0.6.1
- Note workflow instance vs prototype cleanup
- Bump version to 0.6.2
- Adopt gpui-kit 0.6.1 facade
- Bump workspace version to 0.7.0
- Move macOS plist into assets; update README
- Clarify review workflow start log messages
- Bump workspace version to 0.7.1
- Sync Cargo.lock after 0.7.1 bump
- Bump workspace version to 0.7.2
- Bump version to 0.7.3
- Bump version to 0.7.4
- Bump version to 0.8.0
- Bump workspace version to 0.9.0
- Bump workspace version to 0.10.0
- Bump workspace version to 0.11.0
- Add cargo-dist releases and PR-only multi-OS CI
- Add script/release for end-to-end shipping


## [unreleased]

### Features

- Add scrollable multi-channel waveform viewer
- Add application menu with About and Quit
- Show in-window app menu on Windows and Linux
- Add waveform selection model and annotation UX
- Add cpal playback with transport and playhead sync
- Add optional launch, open dialog, and drag-drop
- Add macOS app bundle and CLI install
- Add JSON keymap and view frame commands
- Add View menu and hover ghost playhead
- Add Windows GUI exe and CLI install script
- Add centered Lucide icon transport bar
- Capture bit depth from loaded audio files
- Add status bar with loaded file metadata
- Add clip composition model and Edit menu
- Show generic progress while building waveform peaks
- Dock the waveform beside a collapsible Edits pane
- Highlight edit ranges and restore original media on Init
- Scroll to an edit on click, jump on double-click
- Adopt a custom TitleBar with an embedded menu
- Open multiple compositions in session tabs
- Add Lua scripting with a Script dock
- Save and open .facomp projects and render audio
- Add composition markers and speed up project open
- Add FieldAssist mark as the application icon
- Track composition dirtiness from the clean baseline
- Prompt to save unsaved compositions on close and quit
- Rebrand as FieldAssist 0.2.0
- Rename History dock to Detail
- Persist named collections and marker type definitions
- Add scriptable channel layouts and muted chrome
- Add Lua logging API and Messages tab
- Add Faust monitor chains and a Monitor tab
- Polish Monitor controls and status-bar speaker toggle
- Add B-format AmbiX and FuMa monitor layouts
- Pin monitor params and polish composition tabs
- Seek transport without stopping playback
- Add a session output device picker
- Persist Session as .fasession and expose it to Lua
- Rebuild file drag-drop as Lua workflows
- Add stateful Lua workflows with a session toolbar
- Add a Workflow menu after View
- Group compositions in the explorer
- Add workflow toolbar widgets and pane show/hide commands
- Associate .facomp and .fasession on macOS
- Advance to next todo when marking reviewed
- Use circle-play for status bar preview
- Show git revision in About and add field-assist CLI
- Add FieldAssist app menu and File menu
- Reorder session documents via explorer and Lua
- Rebuild Lua workflows on prototype/instance pattern
- Restyle markers and regions list panels
- Rework monitor panel with RMS meters and Output/Gain
- Run monitor DSP on the realtime callback
- Break out selection into child compositions
- Improve compositions explorer ergonomics
- Surface playback xruns in the Messages panel
- Improve composition break-out naming and explorer cues
- Show message counts on the session status bar
- Clear status-bar message counts on Shift-click
- Add initial pull-based audio analysis
- Add Spectrum view with progressive analysis streams
- Port MinMaxOp and fix spectrum region overlays
- Add Peaks + Spectrum shared lane view
- Regional analysis recompute after edits
- Tint monitor status icon red when output is faulted
- Report output open failures in Messages and status bar
- Open Finder and Dock documents via macOS Apple Events
- Share analysis I/O across peaks and spectrum
- Add macOS Hide, Hide Others, and Show All
- Improve Messages table and composition list display
- Address media by content hash in shared session stores
- Play media files through field-play
- Add shared media pool Lua API and Media dock
- Add View Follow Playhead toggle
- Add File Close Session
- Expand directories in the Add drag-drop workflow
- Move Zero Crossing into the Selection menu
- Break out compositions from regions or channels
- Add waveform peak and spectrum scale gutters
- Add app.ui toolbar controls and Review Keep chrome
- Add Lua theme selection and vendor GPUI themes
- Add Script and Media toggles to the status bar
- Extract field-scripting and field-batch Lua hosts
- Enrich audio device listing and install CLI tools
- Show version and git revision in field-batch REPL
- Lock down Lua require paths with opt-in system modules
- Migrate FieldAssist onto shared field-scripting host
- Drive field-play monitor chain via detect_layout
- Load shared embedded init.lua in field-batch
- Add interactive transport UI to field-play
- Add markers, notes, and save to field-play
- Toggle looping with l in field-play
- Support native Lua C modules for field-batch
- Install field-play and field-batch on Windows
- Expand Lua layout registry and media APIs
- Aggregate open failures into a load-problems dialog
- Add waveform quick note on n
- Add Settings window backed by settings.json
- Add experimental feature flags and gate Analyze
- Rename Render to Export and add Lua export profiles

### Bug Fixes

- Enable File Open from the macOS menu bar
- Restart playback from the start at buffer end
- Keep playhead visible at last sample
- Keep playback stable while hovering edit cards
- Keep the UI responsive while opening large files
- Hide the title-bar app icon on macOS
- Keep playhead and peaks aligned after inserts
- Keep the compositions list closed at startup
- Keep the UI responsive after edits and History
- Scope Space play/pause to the waveform
- Zoom the scroll wheel around the pointer
- Make the loop toggle fill stronger than hover
- Inset waveform channel labels from the header
- Ignore layout and monitor when marking dirty
- Default save name to basename.facomp
- Default session save name from workflow id
- Avoid nested WorkflowBar update on path edit
- Stabilize workflow toolbar path field editing
- Inset macOS status bar from rounded window corners
- Log Lua timings with two decimal places
- Clamp side docks to tab-bar minimum while dragging
- Follow Windows and Linux menu conventions
- Keep decode and pager I/O off the audio callback
- Match REPL input style to history transcript
- Make explorer end-of-list drop reorder reliably
- Clear unused warnings and prefetch monitor tests
- Stop Faust from thrashing monitor rebuilds
- Paint session waveforms as peak blocks land
- Wrap loop playhead and bind playback after session open
- Seek to region start when selecting during play
- Cut idle CPU when monitor pane is open
- Keep Space play/pause after waveform hover
- Tint VU meters with theme chart_3 blue
- Use 5ms attack on Envelope Peak follower
- Clip spectrum tiles on horizontal shrink
- Use radio dots for macOS waveform View items
- Use bandlimited SRC on playback prefetch
- Keep waveform representation app-global
- Keep macOS menus working with no editor window
- Timeout CPAL output open and allow disabled engine
- Fall back to disabled playback on open timeout
- Omit custom title bar in fullscreen
- Use platform-native quit menu labels
- Keep session composition ids aligned with files
- Stabilize Follow Playhead scroll and peaks paint
- Keep waveform hover under the pointer while scrolling
- Hide composition reorder marker during dock resize
- Make Clear consistent and idempotent in EDL history
- Keep selection when toggling Reviewed
- Limit dB/Hz hover to the waveform canvas
- Avoid RefCell panic when Lua sets active composition
- Print field-batch REPL banner before init logs
- Install block decoder when headless hosts open media
- Simplify field-play unsaved quit prompt to y/n
- Resolve Lua media fields in the live desktop backend
- Surface Lua runtime errors instead of masking them
- Make FLAC renders readable in FieldAssist
- Clear monitor chain on channel break-out
- *(ci)* Install ALSA on Linux and skip play test without device
- *(ci)* Import cpal HostTrait in field-play CLI test
- *(ci)* Harden headless audio skip and Windows scripting tests

### Documentation

- Add MIT license, README, and SPDX headers
- Clarify 80-column wrapping for commit messages
- Add TODOs for render, facomp tests, and reload
- Add as-built product specifications
- Move BUILDING.md into docs/
- Tidy README doc links and crate table
- Add app screenshot to README
- Warn that FieldAssist is under development
- Note supported platforms in README
- Fix README warning admonition formatting
- Note Ubuntu packages required to build
- Require cargo fmt before committing code
- Add draft analysis product specification
- Document app.ui toolbar constructors and icons
- Update workflow spec for Keep groups and toolbar API
- Note Review Keep groups in the application spec
- Specify Ingest → Review → Catalog multi-step workflows
- Add structured Review example to field-recording pipeline
- Document field-batch and stop calling field-play an example
- List supported codecs in CLI help
- List decode formats and codecs in README and CLI
- Clarify version-tags ruleset behavior

### Performance

- Keep waveform paint and playback off the pager hot path

### Refactor

- Extract WaveformDisplay with WaveformDataProvider
- Move About to Help menu
- Split FieldAssist into a Cargo workspace
- Genericize field-ui-components behind host traits

### Styling

- Rustfmt play icon asset include
- Mute header and transport icon buttons
- Wrap previous_anchor_near on one line
- Color Messages info logs green
- Shorten About label and bold app menu title
- Rustfmt import order and wrapping

### Miscellaneous Tasks

- Upgrade symphonia to 0.6
- Upgrade cpal to 0.18
- Upgrade mlua to 0.12
- Upgrade ogg to 0.9
- Add SPDX headers to monitor DSP files
- Bump version to 0.4.0
- Note grouping monitor chain controls
- Note MSIX packaging and file-type associations
- Finish renaming snd-review to FieldAssist
- Note workflow toolbar command/callback cleanup
- Bump version to 0.6.0
- Bump version to 0.6.1
- Note workflow instance vs prototype cleanup
- Bump version to 0.6.2
- Adopt gpui-kit 0.6.1 facade
- Bump workspace version to 0.7.0
- Move macOS plist into assets; update README
- Clarify review workflow start log messages
- Bump workspace version to 0.7.1
- Sync Cargo.lock after 0.7.1 bump
- Bump workspace version to 0.7.2
- Bump version to 0.7.3
- Bump version to 0.7.4
- Bump version to 0.8.0
- Bump workspace version to 0.9.0
- Bump workspace version to 0.10.0
- Bump workspace version to 0.11.0
- Add cargo-dist releases and PR-only multi-OS CI
- Add script/release for end-to-end shipping
