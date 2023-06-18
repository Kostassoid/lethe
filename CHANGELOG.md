# Change Log

## [Unreleased] - ReleaseDate

* Fixed known issues in external dependencies.

## [v0.8.0] - 2022-09-25

### Added

* Experimental Apple Silicon (M1) support

### Changed

* MAJOR dependencies upgrade

## [v0.7.0] - 2022-07-11

### Added

* It's now possible to specify starting offset when wiping a storage. Useful for retrying and handling some tricky situations.

### Changed

* Fixed known issues in external dependencies.

## [v0.6.1] - 2021-11-25

### Changed

* [Windows] Locked devices are now correctly skipped during device enumeration.

## [v0.6.0] - 2021-08-15

### Added

* All mounting points associated with a device get unmounted before wiping, including nested partitions/volumes. Previously, this caused issues on Windows, for example, when it was impossible to access a physical drive for writing if it had any mounted volumes.
* Volume label is now part of the storage device description.

### Changed

* Storage devices are now presented as a tree instead of a flat list. This allows to quickly understand the dependency between storage devices.
* Elevated privileges are not required for listing the devices.
* [macOS] Device enumeration implementation was replaced with the one based on 'diskutil'. There upside is that 'sudo' is not required for the 'list' command and that the data is more complete. The downside is that this method is pretty slow and there is a bug in 'diskutil' tool not returning a correct storage size for APFS volumes.
* [linux] Device enumeration implementation was replaced with the one based on 'sysfs' abstractions. Mostly to get the accurate device hierarchy info.
* The progress bar is red now, because DANGER.

## [v0.5.1] - 2021-04-15

### Added

* [Win] Warn when application is running without Administrator privileges.

### Changed

* [Win] Drive geometry info is used when alignment info is not available.
* More detailed errors on hardware failures.

## [v0.5.0] - 2020-12-30

### Added

* Experimental support for detecting/skipping bad blocks.
* Short derived Device IDs.
* More detailed information before and after wipe.
* A "badblocks" inspired wiping scheme.

### Changed

* Increased default block size to 1 MB.

## [v0.4.0] - 2020-06-10

### Added

* Configurable retries.

### Changed

* Improved UI.

## [v0.3.3] - 2020-06-07

### Changed

* Release binaries as archives to preserve permissions.

## [v0.3.0] - 2020-06-06

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
