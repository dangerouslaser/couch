# Release licensing inventory

Couch’s Rust workspaces already declare `GPL-3.0-or-later` in their Cargo
metadata. The repository’s `COPYING` supplies that license text; adding it does
not change those declarations. Slint’s GPLv3 option supports embedded use.
See [Slint licensing](https://slint.dev/pricing) and the
[GNU GPLv3 text](https://www.gnu.org/licenses/gpl-3.0.html).

Keep component boundaries explicit in the release bundle:

- Couch-authored Rust packages: existing GPL-3.0-or-later declarations.
- Linux kernel: retain its existing GPLv2 license and corresponding source.
- BusyBox and other userland packages: retain their individual licenses,
  source obligations, copyright notices and exact version/build records.
- Fonts, icons, artwork and the IR catalog: retain the notices already shipped
  with those assets; do not relabel them under Couch’s code license.
- Vendor firmware and Android runtime files: not covered by Couch’s license.
  Private testing inputs must not be copied into a public release by default.

A top-level license file is not a complete binary release notice or source
archive. Before publication, inventory the actual selected binaries and assets,
include their notices and corresponding source/build information, and verify
that the installed image can be rebuilt from the published inputs. The private
installer trial does not establish redistribution permission for vendor files.
