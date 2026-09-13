# Agent Guidelines

## Commits

This repository uses [Conventional Commits](https://www.conventionalcommits.org/).

Write commit messages in the imperative mood, for example:

- `feat: add stereo waveform lanes`
- `fix: keep drag scrolling after pointer leaves view`

Hard-wrap every line of the commit message — subject and body — to 80
characters or fewer. Do not leave the body as a single unwrapped paragraph.

Before committing any code:

1. Run `cargo fmt` so formatting stays consistent.
2. Run `cargo build` and `cargo test` and ensure both complete with **no
   errors and no warnings** (including unused imports and dead code).
3. Ensure **all tests pass**.

## Workspace

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the crate DAG. Prefer
depending on leaf crates and implementing their traits in the application
rather than introducing sibling-crate edges.

## Realtime audio

Code on the CPAL output callback must not allocate or take blocking locks.
See [crates/field-audio-playback/AGENTS.md](crates/field-audio-playback/AGENTS.md)
for the quality gates and where prefetch vs callback work belongs. Prefetch owns
provider reads and SRC; the callback owns monitor DSP so live parameters track
the audible playhead. Do not use `dasp::ring_buffer` across that boundary (it
requires `&mut self`).
