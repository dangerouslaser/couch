# Frozen desktop launcher acceptance

This fixture runs the exact generated `install.ps1` and real frozen host/TUI
binaries in a Windows ConPTY, selects Cancel, and requires exit 0 without an
installer session. It does not install an OS or contact a device.

The manual workflow requires an independently recorded binary run ID, public
configuration bytes/hash, and final launcher hash. Build receipts must identify
`c09bb5a26cd22fbfa6425e8ec20872e37d673345`; the fixed release version is
`v0.1.0-alpha.24`. The frozen generator must reproduce the supplied launcher hash.
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
