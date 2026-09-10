# Known-device stock identity recapture

`tools/installer/capture_stock_identity.py` captures a fresh private identity
baseline after stock Android has run. Android can legitimately update `nvdata`;
comparing every calibration byte to the older Couch snapshot would then reject
the same physical device. This command preserves the prior baseline and verifies
CID, storage capacity and the complete reviewed partition layout before reading
fresh identity bytes. It never writes flash. DA exit is an explicit option.

This is **not public enrollment of an unknown remote**. It requires a retained,
trusted complete baseline for this unit, its explicitly confirmed CID SHA-256,
a SHA-256-pinned reviewed private `stock-baseline` manifest, the reviewed
mtkclient checkout, and pinned loader and same-device original board-data files.
The manifest is used only for its reviewed model/layout profile; stock OS image
files need not remain present. Matching GPT/MT6580 alone does not identify HA100.

Example (all paths and hashes must be supplied from private reviewed artifacts):

```sh
python3 tools/installer/capture_stock_identity.py \
  --baseline /private/retained-baseline.json \
  --confirm-cid-sha256 "$TRUSTED_CID_SHA256" \
  --stock-manifest /private/stock/manifest.json \
  --stock-manifest-sha256 "$REVIEWED_STOCK_MANIFEST_SHA256" \
  --checkout /private/mtkclient \
  --loader /private/loader.bin --loader-sha256 "$LOADER_SHA256" \
  --preloader /private/preloader.bin --preloader-sha256 "$BOARD_DATA_SHA256" \
  --backup-dir /private/new-stock-identity \
  --bus 1 --ports 2.1 --check-only
```

`--check-only` validates files without USB. Remove it only for the explicitly
planned read-only DA capture, then reboot the connected remote into preloader.
Add `--boot-after-capture` to request DA exit only after independently verified
capture. The HA100 may otherwise fail CDC driver reattachment while still in DA
mode. The tested exit powers the unit off; hold Power to start Android afterward.
An acknowledged exit does not verify OS startup. Failed captures never request
exit; failed exit or cleanup prevents publication of the new baseline.

Physical bus/port selection remains exact. DA upload changes RAM but no flash
write API is exposed. No Couch SSH or running Linux session is needed.

All five identity partitions are copied and independently reread from the device.
Only after successful capture and USB cleanup is `baseline.json` created inside
the new backup directory. It retains the canonical trusted CID only after the
observed DA CID matches the existing fixed encoding. It records new identity
hashes, the predecessor baseline hash, and source pins. An error leaves no usable
new baseline; retain partial backups for diagnosis and retry into a new directory.

Neither vendor Device ID nor MAC is decoded or inferred. Android startup and
identity behavior are not certified by capture; those require separate physical
checks. The public installer gate is unchanged. Use the new baseline explicitly
for subsequent private installation, retaining both old and fresh originals.
