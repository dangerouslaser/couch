# Clean runtime payload inventory

`tools/release/runtime_inventory.py BASE_SPEC NEW_OUTPUT` audits explicit local build artifacts and emits `payload-inventory.json`. When the clean Couch portion passes, it also emits a ready-to-use `staging-input.json` for [offline package assembly](releases.md#assemble-offline-packages). It never contacts the remote, reads personal configuration, executes vendor extraction, or copies a vendor tree into the image.

```sh
python3 tools/release/runtime_inventory.py build/alpine-staging-input.json build/runtime-payload
python3 tools/release/prepare_rootfs.py build/runtime-payload/staging-input.json \
  build/offline-armv7 build/packaged-runtime
```

`BASE_SPEC` supplies the pinned clean Alpine input and timestamp. Its artifact list is replaced by the explicit runtime list. Run on the checkout holding the ARM build artifacts; when transferring to Ollie, preserve those relative source paths and hashes. The inventory's Git commit describes the audited checkout, **not** an attestation that existing binaries were built from that commit. Tracked payload changes are represented only by a cleanliness flag and diff hash; their contents are not copied into the report.

## Runtime files

| Role | Files and checks |
|---|---|
| GUI, configuration server, boot console | `couch-gui`, `couch-confd`, `fbcon`; little-endian ARM32 ELF, no dynamic-loader/library requirement |
| Runtime scripts | `stage2.sh`, `confd.sh`, `setup-mode.sh`, `portal.sh`, `setup-watch.sh`, `wifi-conf.sh`, `sshd.sh`, `confirm.sh`, `scanjson.sh`, `join.sh` |
| Recovery portal | `www/index.html` and exactly `cgi-bin/{save,setpw,scan,enroll}` |
| Readable notices | Lato/Inter OFL and Lucide ISC notices under `/opt/couch/licenses` |
| GUI resources | Lato fonts, Slint source/assets and build-policy hashes; fonts/images are embedded by Slint, and the raw Lucide alpha catalog is verified inside the GUI binary |
| Configuration webUI | Every current `web/couch-web/dist` file must occur verbatim in the daemon binary, including HTML, JavaScript and WASM; a placeholder or stale bundle fails |

Provider clients are Rust libraries linked into GUI/daemon binaries; standalone provider CLIs are not required for normal operation. The diagnostic WMT script is omitted. No settings, network profiles, user configuration, enrollment keys, host keys, properties snapshot, device identity or calibration enters the generated artifact list. The clean stager still generates empty onboarding configuration.

Both existing Lato font files matched upstream Google Fonts binaries; the previously missing OFL notice is now included. The inventory also records offline, locked Cargo package/license metadata for the ARM target, including build dependencies. This is a declared-license inventory, not automatic approval of a license choice or a binary-reachability analysis. In particular, Slint 1.17.1 declares GPL/Slint license alternatives; the release needs an explicit reviewed route and complete dependency notices.

## Vendor and boot inputs remain separate

The vendor report hashes expected WMT binaries, stock recovery modules, firmware patches, Bionic loader/libraries and SELinux property contexts. It records the entire existing firmware/context directory contents for review. Their presence does not establish redistribution permission; vendor files remain rejected by clean artifact staging.

The audited local bundle lacks:

- `vendor/etc/selinux/nonplat_property_contexts`
- `system/etc/selinux/plat_property_contexts`
- `system/etc/ld.config.txt`

Source inputs are the original `vendor.img` and `system.img` backups. Before accepting an extraction, record their verified hashes/origin, exact extracted-file hashes and the applicable redistribution/source/notice evidence. A mixed Android/vendor bundle must not be assigned one invented blanket license. Review its library dependencies and firmware selection; the historical extractor includes more than just the Wi-Fi files.

The development extractor now processes `SYSTEM_DIRS`, retains nested `vendor/etc/selinux`, and creates `system/etc` before dumping its linker configuration. A fixture exercises the actual shell with fake Docker/debugfs and confirms all three paths. This does **not** make the old extractor a release tool: it still has a live-device fallback, a mutable builder tag, and incomplete-input warnings. The inventory does not invoke it.

Boot/recovery candidate images are inspected in memory for Android v0/zImage structure, complete newc bounds, private or appended ramdisk payloads, and current init/BusyBox/fbcon/boot-health members. Their hashes and source-file hashes are recorded. No image is repacked or flashed. Still required: the selected kernel's matching source/config/compiler manifest, DTB/header provenance, reproducible BusyBox/fbcon sources, correctly sized boot and recovery images, and corresponding-source/notice review. Existing normal/diagnostic kernel output manifests are candidates, not proof of the latest deployed kernel; select and verify the exact intended artifacts on Ollie.

## Validation and release status

The audit found 21 clean runtime artifacts, verified all 2,087 current web assets in the daemon, and inventoried 470 GUI/179 daemon dependency records. On Ollie this payload assembled into a 1,301-entry rootfs and a full-size userdata ext4 file that passed `e2fsck`. The resulting archive contains the actual Couch GUI/daemon/scripts, replacing the earlier Alpine+CGI-only fixture; vendor inputs are still absent.

The inventory reports `clean_runtime_ready` separately from `payload_complete` and `installable`; the latter remain false. Clean source-to-binary attestations, vendor completeness/provenance/rights, project/dependency license review, a signed complete release inventory, and physical boot/recovery validation remain gates. The user's existing source changes were preserved, and no physical device interaction occurred.
