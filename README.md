# FieldAssist

Review, edit, and process audio coming in from the field.

![FieldAssist screenshot](docs/asset/screenshot.png)

> [!WARNING]
> **FieldAssist** is currently in development and incomplete. The
> APIs, feature set, and packaging may change at any time. Development happens
> on `main` so the latest version may not be stable.

- See [docs/BUILDING.md](docs/BUILDING.md) for build, packaging, and app-icon generation.
- See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the Cargo workspace crate
map and composition patterns.
- See [docs/SCRIPT.md](docs/SCRIPT.md) for details on the evolving Lua scripting layer.

Supported formats include WAV, FLAC, MP3, OGG, and M4A (via Symphonia).
Supported platforms include Linux, macOS, and Windows.

Open a file from the app with **File → Open…**, or drag and drop onto the
window. Press Space to play or pause.

## Workspace crates


| Crate                  | Role                                              |
| ---------------------- | ------------------------------------------------- |
| `field-core`           | File URLs and job progress                        |
| `field-audio-model`    | PCM buffers, pager, regions, markers              |
| `field-audio-io`       | Probe, decode, encode                             |
| `field-audio-process`  | Offline peaks and resampling                      |
| `field-audio-monitor`  | Listen-path Faust DSP                             |
| `field-audio-playback` | Realtime playback engine                          |
| `field-ui-components`  | Reusable GPUI widgets and traits                  |
| `field-composition`    | `.facomp` / EDL                                   |
| `field-session`        | `.fasession`                                      |
| `field-play`           | Example CLI: play `.facomp` on the default device |
| `FieldAssist`          | Desktop application                               |




### Example: play a composition from the CLI

```bash
cargo run -p field-play -- path/to/project.facomp
```

`field-play` opens the `.facomp`, uses its monitoring chain when set, and
otherwise sends audio Direct to the system default output device.

## License

MIT — see [LICENSE](LICENSE).