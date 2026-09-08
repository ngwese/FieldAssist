# TODO

- [ ] Fix "flac" render output
- [ ] Build roundtrip tests for composition files
- [x] Decide on dirty indicator and history semantics when
      (re)loading a composition
- [ ] Profile composition loading when loading a large FLAC file;
      determine stall when the file must be probed
- [ ] Find the performance regression when loading a composition
      that has edits
- [ ] Group and order monitor chain controls (see stereo
      crossfeed)
- [ ] Package Windows app as MSIX and declare supported file
      types in the application manifest
- [ ] Clean up inconsistent command vs callback structure on
      workflow toolbar controls (buttons use `command` +
      `:on("command", …)`; path/toggle use inline `on_path` /
      `on_change`)
- [ ] Fix long-running workflows to use a workflow instance
      instead of mutating the prototype (toolbar, output path,
      and other run state today live on the shared prototype)

