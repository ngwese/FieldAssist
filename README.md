# FieldAssist

Review, edit, and process audio coming in from the field.

See [BUILDING.md](BUILDING.md) for build, packaging, and app-icon generation.
See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the Cargo workspace crate
map and composition patterns.

Supported formats include WAV, FLAC, MP3, OGG, and M4A (via Symphonia).

Open a file from the app with **File → Open…**, or drag and drop onto the
window. Press Space to play or pause.

## Workspace crates

| Crate | Role |
| --- | --- |
| `field-core` | File URLs and job progress |
| `field-audio-model` | PCM buffers, pager, regions, markers |
| `field-audio-io` | Probe, decode, encode |
| `field-audio-process` | Offline peaks and resampling |
| `field-audio-monitor` | Listen-path Faust DSP |
| `field-audio-playback` | Realtime playback engine |
| `field-ui-components` | Reusable GPUI widgets and traits |
| `field-composition` | `.facomp` / EDL |
| `field-session` | `.fasession` |
| `FieldAssist` | Desktop application |

## License

MIT — see [LICENSE](LICENSE).
