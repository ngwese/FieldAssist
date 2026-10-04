# Export Tags Specification

## Purpose

Catalog exports write review and source metadata into library files,
mapping BWF/iXML fields onto stable FLAC Vorbis keys.

## Requirements

### Requirement: Carry bext and iXML into FLAC Vorbis

When Catalog (or composition render) encodes a FLAC library file, the
system SHALL map known `source.bwf` and `source.ixml` leaves into stable
Vorbis comment keys and write them on the encoded file. Unknown or empty
leaves SHALL be omitted. The mapping SHALL cover at least the host
canonical BWF/iXML-related keys used at export time (description,
originator, originator reference, origination date/time, time reference,
coding history, project, scene, take, tape, note, and iXML leaf values
present on the source).

#### Scenario: BWF description becomes Vorbis comment

- **GIVEN** a keep composition whose media has `source.bwf.Description`
- **WHEN** Catalog exports FLAC
- **THEN** the FLAC file contains a stable Vorbis key for that description

#### Scenario: Empty tags omitted

- **GIVEN** a source with no bext/iXML values
- **WHEN** Catalog exports FLAC
- **THEN** encode succeeds without inventing placeholder tag values

### Requirement: User artist and copyright on catalog files

Catalog exports SHALL write `user.artist` and `user.copyright` into the
catalog format’s artist and copyright fields when those variables are
non-empty (Vorbis `ARTIST` / `COPYRIGHT` for FLAC).

#### Scenario: Artist and copyright injected

- **GIVEN** `user.artist` and `user.copyright` are set
- **WHEN** Catalog exports a keep composition as FLAC
- **THEN** the file’s ARTIST and COPYRIGHT tags match those values

#### Scenario: Missing user fields skipped

- **GIVEN** `user.artist` is empty
- **WHEN** Catalog exports FLAC
- **THEN** no empty ARTIST tag is forced from that variable
