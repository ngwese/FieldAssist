# Agent Guidelines

## Commits

This repository uses [Conventional Commits](https://www.conventionalcommits.org/).

Write commit messages in the imperative mood, for example:

- `feat: add stereo waveform lanes`
- `fix: keep drag scrolling after pointer leaves view`

Hard-wrap every line of the commit message — subject and body — to 80
characters or fewer. Do not leave the body as a single unwrapped paragraph.

## Workspace

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the crate DAG. Prefer
depending on leaf crates and implementing their traits in the application
rather than introducing sibling-crate edges.
