# Change Log

## [Unreleased] - ReleaseDate

### Added

* Windows support.
* List of devices now includes more information (storage type, mount points).

### Changed

* Default IO block size is now 64 KB regardless of reported device block size.

## [v0.2.1] - 2019-09-23

### Fixed

* Fixed verification stage on Linux.

### Changed

* Improved error messages, especially for WSL.

## [v0.2.0] - 2019-09-16

This release is mostly laying the groundwork for a more serious improvements.

### Added

* It is now possible to provide block size with a scale unit. E.g. `128k` (128 kilobytes) instead of `131072`. Additionally, the number is checked to be a power of two.

### Changed

* Unsuccessful validation now retries at the last successful position.
