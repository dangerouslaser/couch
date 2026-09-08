# HA100 runtime validation — 2026-09-08

The live normal image is `couch-stable.img`, SHA-256
`48278dc417dd188cc70afabac661039000717d477f07720a34f206f3639909a1`.
Kernel source is `15b9bb349413240abb5538db678a2f9a8c4653ac`; the normal profile
**disables experimental PWM IR**. Kernel compilation ran natively on Ollie.
The same source revision was also tested with IR enabled, so identify artifacts
by their manifest/config/image hashes, not uname alone.

- Production panel timing and keypad EINT configuration were physically
  confirmed by the user. The screen remains undistorted after boot.
- Touch reports preserve complete evdev batches and two-finger slot ownership.
  Raw capture previously observed 14 contacts and 14 matching releases.
  Kernel report reads respect the eight-byte I2C FIFO and retry transient errors.
- The GUI consumes the first dim-screen touch to restore brightness, then allows
  the next contact to select. Full powerdown still requires a physical button.
  Final physical confirmation of this dim-touch change remains pending.
- GUI health gating passed: advancing local heartbeats clear the BCB; a later
  read at uptime 519s confirmed the BCB is zero. Eight Python boot/build tests
  and eighteen GUI unit tests pass. Networking is not a health prerequisite.
- Rescue p9 remains unchanged: full-partition MD5
  `2ac16bf92bf9d92220b8af3f0ea46600`.
- Experimental IR failed a four-byte zero-waveform write: the device hung and
  watchdog-rebooted. Clock ordering did not resolve it. No target-device IR
  control has been validated. The driver is disabled in normal builds, and
  stage2 removes the stale stock `/dev/irtx` node when it is absent.
- GPU EGLImage imports succeed, but shared-buffer tests detect stale pixels.
  No GPU presentation path is enabled in the GUI.

Full unplugged suspend, battery/thermal behavior, IR reception by a target, and
long-duration GPU/allocator stability are separate outstanding hardware tests.
Existing workspace formatting differences mean `cargo fmt --check` is not yet
clean; this work does not claim a repository-wide formatting pass.
