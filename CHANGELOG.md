# Change Log

## [$Unreleased] - $ReleaseDate

This release is mostly laying the groundwork for a more serious improvements.

### Added

* It is now possible to provide block size with a scale unit. E.g. `128k` (128 kilobytes) instead of `131072`. Additionally, the number is checked to be a power of two.

### Changed

* Usuccessful validation now retries at the last successful position.
