# Couch has moved to [Couch-OS/couch](https://github.com/Couch-OS/couch)

Development, issues and new releases are at **https://github.com/Couch-OS/couch**.

This repository holds no source code. It exists only so that Couch systems
installed before the move keep working:

- **Remotes running `v0.1.0-alpha.20260916.170` or earlier** look for updates
  here. They are offered
  [`v0.1.0-alpha.20260917.173`](https://github.com/dangerouslaser/couch/releases/tag/v0.1.0-alpha.20260917.173),
  whose updater finds later releases at the new location by itself. Once that
  update is installed, this repository is never used again.
- **The published `v0.1.0-alpha.20260916.170` install commands** download the
  desktop installer and the OS payload from here.

Nothing new will be published here, and the repository is archived. Please file
issues and pull requests at the new location.

## Source code for the binaries published here

These releases contain binaries built from GPL-licensed source. The complete
corresponding source for each version is published at the new location:

- `v0.1.0-alpha.20260917.173`: source commit
  [`35d6a0c`](https://github.com/Couch-OS/couch/tree/35d6a0c65d89f708be3cf484bfbbd6109ddbc265),
  plus the source archives on
  [its release](https://github.com/Couch-OS/couch/releases/tag/v0.1.0-alpha.20260917.173)
- `v0.1.0-alpha.20260916.170`: source commit
  [`de2c0ec`](https://github.com/Couch-OS/couch/tree/de2c0ecda41102816c9121c421dfdede5b704dbc),
  plus the kernel, BusyBox, BlueZ and installer source archives on
  [its release](https://github.com/Couch-OS/couch/releases/tag/v0.1.0-alpha.20260916.170)

Couch is GPL-3.0-or-later; see
[COPYING](https://github.com/Couch-OS/couch/blob/main/COPYING) at the new
location.
