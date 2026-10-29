# Output Buffer Specification

## Purpose

Defines how playback hosts choose a CPAL output buffer size from a preferred
setting or platform defaults, accept what the device can open, and report the
result. Informed by docs/spec/SPEC-application.md (Audio device / period) and
docs/ARCHITECTURE.md (Linux ALSA host / period selection).

## Requirements

### Requirement: Preferred buffer size is not guaranteed

When `audio.buffer_size_frames` is set to a listed catalog size, the host
SHALL request that size as a preferred Fixed buffer. If the device rejects
it, the host MUST continue with the same negotiation path used when no
preference is set, and MUST NOT fail open solely because the preferred size
was unavailable.

#### Scenario: Preferred size accepted

- **WHEN** the user has set a catalog buffer size and the device accepts that
  Fixed request
- **THEN** playback opens with that preferred size

#### Scenario: Preferred size rejected

- **WHEN** the user has set a catalog buffer size and the device rejects that
  Fixed request
- **THEN** the host negotiates an alternative openable size (including
  platform Default) and playback still opens when the platform allows any
  successful configuration

### Requirement: Unset preference renegotiates on every open

When `audio.buffer_size_frames` is unset, the host SHALL negotiate buffer size
on every stream open and MUST NOT write the negotiated result back into
settings.

#### Scenario: Auto path does not persist

- **WHEN** settings have no buffer size preference and a stream opens
  successfully with a negotiated Fixed or Default size
- **THEN** `audio.buffer_size_frames` remains unset after open

#### Scenario: Explicit preference retained on fallback

- **WHEN** the user has set an explicit preferred size and open falls back to
  another size
- **THEN** the stored preference remains the user's chosen value

### Requirement: Negotiation prefers headroom within platform limits

When negotiating without an accepted Fixed preference, the host SHALL probe
the platform default buffer size, prefer a catalog size of at least twice
that default when the device's supported buffer-size range allows it, and
otherwise MUST try other in-range catalog sizes (larger and smaller than the
preferred target) before accepting platform Default.

#### Scenario: Twice-default fits supported range

- **WHEN** the probed default is D and the supported range includes a catalog
  size ≥ 2×D
- **THEN** the host attempts Fixed open at the smallest such catalog size
  (and may try further in-range sizes if that open fails)

#### Scenario: Larger Fixed sizes unavailable

- **WHEN** sizes at or above 2× the probed default are rejected or outside
  the supported range
- **THEN** the host tries smaller in-range catalog sizes and/or platform
  Default rather than only walking upward

#### Scenario: Constrained host accepts Default

- **WHEN** no Fixed catalog candidate opens and platform Default succeeds
- **THEN** playback opens with the platform default buffer size

### Requirement: Settings key is buffer_size_frames

Settings SHALL store the optional preferred size under
`audio.buffer_size_frames`. Readers MUST accept legacy `audio.period_frames`
as an alias for the same field. New writes MUST use `buffer_size_frames`
only.

#### Scenario: Legacy key loads

- **WHEN** `settings.json` contains `audio.period_frames` and no
  `buffer_size_frames`
- **THEN** the host treats that value as the preferred buffer size

#### Scenario: New writes use renamed key

- **WHEN** the user saves an explicit buffer size preference
- **THEN** the file contains `audio.buffer_size_frames` and does not rely on
  `period_frames` as the written key

### Requirement: UI presents preferred buffer size

The FieldAssist Settings → Audio UI SHALL label the control "Buffer Size",
present listed frame sizes plus an unset/auto choice, and describe the value
as a preferred size that may not match what the device opens.

#### Scenario: Label and description

- **WHEN** the user opens Settings → Audio → Device
- **THEN** they see "Buffer Size" with copy indicating a preferred frame
  count, not a guaranteed period

### Requirement: Open logs actual buffer size

After a successful stream open, the host SHALL log the actual buffer size in
frames obtained from the opened stream (including when platform Default was
used).

#### Scenario: Fixed open logs actual size

- **WHEN** a stream opens with a Fixed buffer request
- **THEN** logs include the actual buffer size reported for that stream

#### Scenario: Default open logs actual size

- **WHEN** a stream opens with platform Default
- **THEN** logs include the actual negotiated buffer size, not only that
  Default was requested
