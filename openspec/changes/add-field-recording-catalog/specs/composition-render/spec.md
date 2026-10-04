# Spec Delta

## Purpose

Multi-output composition render amortizes one source read across N encoded
outputs and applies optional offline DSP chains per output.

## ADDED Requirements

### Requirement: Multi-output render plan

The system SHALL provide a composition render operation that accepts one
source composition and a list of one or more outputs. Each output SHALL
specify a destination path, an export profile (or equivalent encode
settings), and an optional DSP chain identifier with optional parameter
map. The implementation SHALL read composition audio once per render call
(amortized across outputs) rather than re-decoding the full source once
per output when multiple outputs are requested. After that shared read
(and any shared DSP stage), per-output channel select / offline DSP /
encode SHALL run concurrently across outputs. Async render jobs SHALL
expose progress in source frames (`done` / `total` = composition frame
count), not output-file counts.

#### Scenario: Identity plus stereo sidecar

- **GIVEN** a kept FOA composition and a render plan with an identity
  output and a stereo mixdown output
- **WHEN** render executes
- **THEN** both files are produced from a single amortized source read
  with per-output work overlapping in parallel

#### Scenario: Identity-only plan

- **GIVEN** a mono or stereo composition
- **WHEN** render is invoked with a single identity output
- **THEN** one file is written and no mixdown DSP runs

#### Scenario: Frame progress on async render

- **GIVEN** a composition of N source frames and a multi-output plan
- **WHEN** `begin_render` runs
- **THEN** the job reports `total = N` and `done` advances toward N as
  outputs process frames

### Requirement: Offline Faust mixdown for monitor chains

When an output requests a DSP chain that matches a shipping monitor chain
used for Ambisonic or M/S listen (`foa`, `foa_fuma`, or `ms`), the system
SHALL apply that Faust DSP offline (not on the CPAL output callback) using
fixed chain defaults unless the caller supplies parameter overrides.
Catalog and other callers SHALL use the same chain-selection trigger as
playback layout detection for deciding when a mixdown output is included.

#### Scenario: Default FOA mixdown

- **GIVEN** an output with chain `foa` and no parameter overrides
- **WHEN** offline render runs
- **THEN** stereo PCM is produced with Faust FOA defaults

#### Scenario: Parameter overrides

- **GIVEN** an output with chain `foa` and explicit DSP parameters
- **WHEN** offline render runs
- **THEN** those parameters are applied instead of the fixed defaults

#### Scenario: Realtime path unchanged

- **GIVEN** offline render is running
- **WHEN** playback is also active
- **THEN** the CPAL callback path does not perform the offline render work

### Requirement: Future shared DSP slot

The render plan model SHALL allow a shared DSP stage applied to the
decoded source before per-output DSP, even if Catalog v1 leaves that stage
empty. Per-output DSP SHALL still run after any shared stage when both are
present.

#### Scenario: Empty shared stage

- **GIVEN** a render plan with no shared DSP and two per-output chains
- **WHEN** render executes
- **THEN** each output receives only its own per-output DSP after decode
