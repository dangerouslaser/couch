# Installer terminal interface

This independent Rust workspace builds the Ratatui terminal frontend. It does
not contain the installer backend, device payloads or host USB dependencies.

```sh
cargo run -- --demo
cargo run -- --snapshot
cargo test --locked
```

Both preview modes work without a device. Normal operation takes explicit
`--python`, `--backend`, and `--config` paths. The backend supplies structured
progress and input requests; Unix uses an inherited socket and Windows uses
private standard-input/output pipes. Pairing or Wi-Fi secrets are masked in the
terminal and must not be included in logs.

The build workflow produces Linux x64/ARM64, Windows x64 and macOS Intel/ARM
binaries, plus a universal macOS binary. Current Linux artifacts require glibc
2.39. macOS artifacts use ad-hoc signing. Windows backend IPC and cancellation
still need end-to-end validation; native UI tests are not USB installation tests.
A complete user-facing installer package and platform launcher are separate work.

![Device-free terminal preview](preview.png)
