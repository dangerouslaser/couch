# Repository Guidelines

## Project Structure & Module Organization

Couch is a Linux distribution for the Sanytron Astrion HA100 remote (ARMv7, Alpine userland).

- `model/`, `clients/`, `daemon/`, `ui/`, and `web/` are separate Rust workspaces: shared configuration types, Kodi/voice/IR integrations, configuration server, Slint device GUI, and browser configuration UI respectively. Preserve their independent lockfiles.
- `initramfs/init`, `recovery/init`, and `stage2/` implement boot, rescue, and runtime setup; `stage2/www/` contains the setup portal.
- `kernel/` holds kernel build configuration; `tools/` contains build, deployment, and diagnostic scripts.
- `src/` contains C framebuffer utilities; `gui/` contains the LVGL implementation. Assets live in `assets/` and UI subdirectories. Consult `docs/` for subsystem details.
- Keep maintained architecture, usage, recovery and validation procedures in `docs/`. Put session handoffs, dated work logs, private trial records and disposable plans in the gitignored `scratchpad/` directory. Promote lasting findings into the relevant guide instead of appending a session transcript.

## Build, Test, and Development Commands

Run these from the repository root unless indicated:

- `tools/build-webui.sh --host`: build the browser bundle, then the host configuration daemon. Requires Trunk and the `wasm32-unknown-unknown` Rust target.
- `tools/run-webui.sh`: serve the local configuration UI at `http://127.0.0.1:8090` using `build/couch-config.json`.
- `tools/build-webui.sh`: build the bundle and ARMv7 daemon for deployment.
- `(cd clients && cargo build --release --target armv7-unknown-linux-musleabihf)`: cross-compile integration clients using workspace linker configuration.
- `tools/build.sh`: package the stock kernel and initramfs; requires the device backup configured in `tools/env.sh`.
- `kernel/build.sh`: build the kernel in Docker; follow `kernel/README.md` for prerequisites.

## Coding Style & Naming Conventions

Use four-space indentation in Rust and Python; match surrounding C, shell, and Slint formatting. Rust uses `snake_case` functions/modules and `PascalCase` types. Keep shell scripts compatible with their declared interpreter. Use `cargo fmt --check` within the affected Rust workspace; no repository-wide lint configuration is provided. Explain hardware assumptions near ABI constants and device operations.

## Testing Guidelines

Run `cargo test` inside each affected workspace, particularly `model/`, `daemon/`, and `clients/`. Tests use Rust's built-in framework, inline `#[cfg(test)]` modules, and descriptive `snake_case` names. No numerical coverage threshold is specified. Add regression tests for changed logic; record device validation separately for framebuffer, boot, audio, and IR changes.

## Commit & Pull Request Guidelines

History favors descriptive, imperative subjects, sometimes prefixed by a subsystem (`docs:`, `couch-ir:`). Keep commits focused. PRs should explain behavior changes, list validation commands/results, link relevant issues, and include screenshots for UI changes. Update related subsystem documentation.

## Device & Configuration Safety

Keep credentials, partition backups, and per-device calibration outside Git. Never write `preloader_*` or `lk`. Read the current partition layout and recovery procedure in `docs/device-recovery.md` before flashing.
