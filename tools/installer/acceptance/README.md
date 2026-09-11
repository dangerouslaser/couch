# Frozen desktop launcher acceptance

This fixture runs the exact generated `install.ps1` and real frozen host/TUI
binaries in a Windows ConPTY, selects Cancel, and requires exit 0 without an
installer session. It does not install an OS or contact a device.

The manual workflow requires an independently recorded binary run ID, public
configuration bytes/hash, and final launcher hash. Build receipts, the workflow run,
and generator checkout must match the independently supplied forty-digit
`source_commit` host pin (current host:
`57a3e22b4e86d8d6620dbaedbf30847e26b9bbd4`). The configuration must instead match
an independently supplied `payload_source_commit` OS pin (current payload:
`271704728c77c13add1763aea7d1f9bd629c63ca`). Both pins are required workflow inputs;
neither is inferred from downloaded artifacts. This permits rebuilding the installer
without rebuilding an unchanged OS payload. The fixed release version is
`v0.1.0-alpha.20260910.24`. The frozen host-source generator must reproduce the
independently supplied launcher hash. The admission receipt records both source pins.
No executable or launcher is patched for the test.

Before publication, only network routing is substituted: an ephemeral hosted
Windows runner maps `github.com` to loopback and trusts a temporary fixture
certificate through the normal Windows trust store. An HTTPS listener serves
exactly the three admitted assets in their expected order. The runner's hosts
file, certificate store, and SSL binding are restored afterward. These fixture
changes are never made on a developer or user machine. The script intentionally
refuses to run outside a GitHub-hosted Actions runner.

The result records the admitted source/binary/config/launcher hashes, exact three
requests, ConPTY transcript, exit status, and absence of a native session.
Physical driver binding and installation acceptance remain separate tests.

The ConPTY child explicitly clears inherited redirected standard handles, as
[described by the Windows Terminal maintainer](https://github.com/microsoft/terminal/discussions/15814),
so hosted runner pipes cannot replace the actual console.
