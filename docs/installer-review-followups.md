# Installer review follow-ups (PR #74, 2026-09-12)

A high-effort multi-angle review of PR #74 confirmed ten correctness findings.
The release-critical one is fixed in this PR; the rest are tracked here because
they live in code paths that the hardware-validated install-from-Android flow
does not exercise (the firmware-watch workflow, the Wi-Fi debug tooling, and the
not-yet-validated Wi-Fi stock-restore feature).

## Fixed in this PR

- **Host/Python Wi-Fi ready timeout was shorter than the stage's new wait.**
  `network::ready` capped at 120 s and `wifi_benchmark.wait_ready` at 60 s, but
  the stage's worst case grew to ~128 s (transport 20 + interface 15 + settle 3
  + 90 s socket wait). Both budgets are now 180 s and the host reports the last
  stage status on timeout instead of a generic message.

## Fixed after the first review pass

- **#10 restore rebind gate** now binds a restore by immutable hardware identity
  (chip, CID encoding, eMMC CID, capacity) and the five calibration partitions,
  and no longer requires the mutable OS partitions or the layout map to match;
  each check is named so a mismatch is diagnosable. `SavedEnrollment::rebind_mode`.
- **#6/#8 firmware-watch** add `sys.path` for the `import` under `shell: python`
  and reject a non-ZIP Drive response before derivation, so the job runs and
  fails soft instead of dying on `ModuleNotFoundError`/`BadZipFile`.
- **#7 firmware-watch auto-PR** no longer writes a half-update that fails its own
  validation; it commits a `firmware-drift-report.md` drift notice (a green,
  reviewable PR) naming the three coupled pins a maintainer must update together.

## Deferred (not on the hardware-validated install path)

1. **Restore plan key and older stages.** The host sends `restore: true` only
   for a restore, so this release's matched stage accepts every plan. A host
   from this PR driving a *pre-PR* stage image would still have that stage reject
   an unknown key; a stage capability flag in the status JSON would let the host
   gate on it. Low risk while host and stage ship together.
2. **Debug supervisor `killall` on the normal retry path.** `killall
   wpa_supplicant` runs only in the worker-still-alive branch; after a
   `supplicant-socket-timeout` (which intentionally leaves the blocked supplicant
   alive) the next generation can race two supplicants for the control socket.
   Debug-stage only.
3. **`$BB wait` is not a BusyBox applet.** `debug-supervisor` calls
   `$BB wait "$worker"`; `wait` is an ash builtin, so `result` is always 127 and
   the `worker_exit` lifecycle field always reads "failure". Cosmetic, debug-only.
4. **Stage debug `lifecycle` made mandatory without a `debug_protocol` bump.** A
   still-running pre-PR debug stage now errors in `lifecycle_summary` instead of
   degrading to read-only diagnostics.
5. **`debug_late_watch` can append a prior generation's record** into the next
   generation's bounded debug log after a retry. Debug-only log hygiene.
6. **firmware-watch.yml: `import firmware_restore` needs `sys.path`.** Under
   `shell: python` the step runs from a temp dir, so the import fails; the watcher
   never detects drift.
7. **firmware-watch.yml: the auto-PR edits only `archive.*`,** which
   `firmware_restore.load()` then rejects against the official runtime pin, so the
   generated PR fails its own validation.
8. **firmware-watch.yml: the fetched OTA is not checked for ZIP magic,** so a
   Drive HTML interstitial reaches `derive()` and raises `BadZipFile`, turning the
   "fail-soft" job red.
9. **`recover_native_bootstrap.recover` chain kwargs are reachable only from
   tests and `recover_debug_chain.py` only prints an admission JSON;**
   `docs/installer-bootstrap-recovery.md` still documents the 4-arg form.
10. **Wi-Fi stock-restore rebind gate is too strict for restore.**
    `SavedEnrollment::rebind` requires live boot/recovery/odmdtbo/logo to still
    equal the enrollment's saved originals. On a device running Couch that can
    never hold for an Android enrollment, so the merged Wi-Fi restore stops at
    "live hardware differs from retained enrollment" before any write. The
    restore path must compare only stable identity (CID, capacity, partition
    layout, the five calibration partitions) and not the mutable OS partitions it
    is about to overwrite. This is why the over-Wi-Fi stock restore is documented
    as not hardware validated; the USB download-agent stock restore is unaffected
    and was exercised on hardware on 2026-09-12.
