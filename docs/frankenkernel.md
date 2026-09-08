# Building a kernel from source (the "frankenkernel")

> **Status, 2026-09-08.** Done through Phase 3's WiFi gate: `3.18.79 #4` boots
> the device from the `boot` slot with display, keypad, USB serial and WiFi,
> and runs all of Couch. Phase 0-2 went as planned except that serial, not the
> panel, was the hard part (the panel came up before the console did, see
> `kernel/README.md`). Still open: the `tlsc6x` touchscreen and `mt_irtx`
> (Phase 3 touch, Phase 4 IR), then Phase 4 proper. Two things below are no
> longer true and are corrected in place: Couch boots from `boot`, not
> `recovery`, and Android is no longer on the device - the recovery slot holds
> Couch's own rescue image (`tools/build-recovery.sh`), and that, not lk's
> menu, is how a bad kernel is survived. The tree, its delta and the build are
> described in `kernel/README.md`; the rest of this document is the plan as
> written, kept because the reasoning still holds.

When this was written Couch ran the stock 3.18.79 kernel byte-for-byte because
the three drivers that make this device a device - the ST7701s display, the
tlsc6x touchscreen and the CONSYS WiFi - exist in no public source tree, and
the vendor has not released theirs. This is the plan to build our own 3.18
kernel anyway, by assembling a MediaTek ALPS `alps-3.18.79` base with those
drivers ported in from other public MTK trees. The prize is a kernel we can read, profile and change: fixing the
46-62ms keypad interrupt handler, turning on real suspend, and owning the
hotplug and display-pacing behaviour we currently fight from userspace.

It is weeks of work with a genuine display-bring-up risk. It is also
integration, not invention: every piece exists somewhere public. This document
is the map.

## Goal and non-goals

**Goal:** a `build/kernel.bin` compiled from source that boots this device with
display, touch, keypad and WiFi working, as a drop-in for the stock kernel in
Couch's existing boot flow (our initramfs; at the time the recovery slot with
Android untouched, now the `boot` slot with Couch's recovery image on p9).

**Non-goals:** a newer kernel (there is no 4.x/mainline path with working WiFi -
see docs and the kernel-research notes), replacing the bootloader (lk/preloader
stay stock, this is the whole safety model), and IR at first (nice-to-have, not
on the boot path).

## Why this is feasible: what we already have

The device backup (`~/Projects/sanytron-device-backup/`) is most of the hard
part already done:

- **`config-stock.txt`** - the exact kernel `.config`, ~3800 lines. This is the
  build's source of truth. It names everything:
  - `CONFIG_ARM=y`, `CONFIG_CROSS_COMPILE="arm-eabi-"` - 32-bit ARM, AOSP arm-eabi GCC.
  - `CONFIG_MTK_PLATFORM="mt6580"`, `CONFIG_ARCH_MTK_PROJECT="x15cm_s90_kr"`.
  - `CONFIG_CUSTOM_KERNEL_LCM="st7701s_wvga_dsi_vdo_boe_tn_tianxian"` (480x800).
  - `CONFIG_MTK_COMBO_CHIP="CONSYS_6580"` - the in-SoC WiFi/BT.
  - `CONFIG_TOUCHSCREEN_MTK_TLSC6X=y`, `CONFIG_MTK_IRTX_PWM_SUPPORT=y`.
- **`boot.img` / `kernel.bin`** - the stock boot image and the extracted kernel.
  `kernel.bin` is a raw ARM `Image` (uncompressed, head is `mov r0,r0` NOPs) with
  the **DTB appended at offset ~6974008** (FDT magic `d00dfeed`). Our build must
  produce the same shape: `Image` + appended DTB.
- **`odmdtbo.img`** - the device-tree overlay lk applies at boot (panel, touch,
  keypad, IR nodes). We already read and patch it (`tools/dtbpatch.py`).
- **`lk.img`** - the bootloader. It contains its own copy of the panel init for
  the boot logo, i.e. the exact ST7701s init sequence, which is our escape hatch
  if the ported LCM driver is wrong (see Display below).
- **Calibration partitions** - `nvram.img`, `nvdata.img`, `protect1/2.img`,
  `proinfo.img`. WiFi MAC and RF calibration live here; the running system
  already uses them, so WiFi has what it needs once the driver builds.
- **Recovery model** - stock `recovery.img`, and Couch boots from the recovery
  slot with Android intact on the other slot. A panicking kernel is one
  power-cycle and an lk menu selection away from a working device.

## The integration point (small change to Couch)

`tools/build.sh` today does:

    [ -f build/kernel.bin ] || unpack the stock boot.img -> build/kernel.bin
    ... build our initramfs ...
    tools/bootimg.py pack build/kernel.bin build/initramfs.cpio.gz -> linux-recovery.img

So the only change Couch needs is a new way to *produce* `build/kernel.bin`: a
`tools/build-kernel.sh` that compiles the tree and emits `Image + appended DTB`.
Everything downstream - initramfs, packing, flashing to the recovery slot - is
unchanged. The frankenkernel is a kernel-build project bolted onto a boot flow
that already works.

## Sources to assemble

| piece | config symbol | where it comes from |
|---|---|---|
| base tree | `MTK_PLATFORM="mt6580"` 3.18.79 | `parthibx24/android_kernel_wiko_k300` (`alps-3.18.79`) - has the MTK display/EINT/keypad frameworks, `connectivity/wlan`, and ST7701s LCM variants |
| display panel | `CUSTOM_KERNEL_LCM="st7701s_wvga_dsi_vdo_boe_tn_tianxian"` | closest public: wiko_k300 `st7701s_wvga_dsi_vdo_tn_boe`; adapt/rename, tune init (or lift the exact init from `lk.img`) |
| touch | `TOUCHSCREEN_MTK_TLSC6X` | `danascape/linux-daria-mt6877` has tlsc6x wired into MediaTek's `tpd` framework; ~20 other trees carry `tlsc6x_main.c` |
| WiFi | `MTK_COMBO_CHIP="CONSYS_6580"` | `drivers/misc/mediatek/connectivity/wlan` in the MT6580 trees (source builds `wlan_drv.ko` to match our ABI) |
| IR (later) | `MTK_IRTX_PWM_SUPPORT` | `mt_irtx_pwm.c` from other MTK trees (already understood - see `docs/ir.md`) |

## Toolchain and build environment

- **Compiler:** AOSP `arm-eabi` / `arm-linux-androideabi` **GCC 4.9** (the ALPS
  3.18 era compiler; modern GCC will not build this tree cleanly).
- **Host:** Linux. The toolchain is `linux-x86`, and a 3.18 kernel build wants a
  Linux host. Use a Docker container on the Mac (e.g. `ubuntu:18.04` + the AOSP
  prebuilt GCC + `make gcc bc libssl-dev`).
- **Build the kernel directory directly, bypass the ALPS wrapper.** The full ALPS
  build system drives everything through `ProjectConfig.mk` keyed on
  `x15cm_s90_kr`, which we do not have. We do not need it: we have the finished
  `.config`. Drop `config-stock.txt` in as `kernel/.config`, then
  `make ARCH=arm CROSS_COMPILE=arm-eabi- oldconfig && make -jN Image dtbs`. The
  ALPS Kbuild pulls the mt6580 platform dirs from `MTK_PLATFORM`.

## Phased plan (each phase has a go/no-go gate)

### Phase 0 - Foundation, zero device risk
1. Extract and archive from the backup: the exact kernel version string and
   cmdline (`tools/bootimg.py` on `boot.img`), the appended DTB from `kernel.bin`,
   and the panel/touch/keypad/IR nodes from `odmdtbo.img`.
2. Clone the base tree; get it to **compile unmodified** with the GCC-4.9 Docker
   env, for any mt6580 defconfig it already has. Deliverable: a green build. This
   proves the toolchain and Kbuild before we touch drivers.
3. Diff the base tree's mt6580 defconfig against `config-stock.txt` - the delta
   is the list of drivers/options we must add.
4. Stand up `tools/build-kernel.sh` producing `Image + appended DTB`.
   **Gate:** the base tree builds, and we can produce a `kernel.bin`-shaped file.

### Phase 1 - Boots to a serial console (no display needed)
1. Build with `config-stock.txt` as `.config` (adjust for the base tree).
2. Reuse the **stock appended DTB** with our `Image` (the DTB describes hardware
   that has not changed; avoids DTB-build issues on day one).
3. Pack with our initramfs; flash **only** the recovery-slot boot image.
4. Target: kernel boots, brings up MMC (mounts our rootfs) and the UART console
   (`ttyMT1`), runs init. We already have serial as the debug channel, so no
   display is required to call this a success.
   **Gate:** a from-source kernel reaches our init over serial and the keypad
   interrupt handler is measurable (confirm it is the fast mainline-style path).

### Phase 2 - The panel (the hard part)
1. Port the `st7701s_wvga_dsi_vdo_tn_boe` LCM driver in as
   `..._boe_tn_tianxian`; set `CONFIG_CUSTOM_KERNEL_LCM` to match.
2. Boot; if the panel is black/garbled, the init sequence is wrong. Recover via
   the lk menu, then **lift the exact init sequence from `lk.img`** (lk drives
   the same panel for the boot logo) and transplant it into the LCM driver.
3. Bring up the framebuffer, then Couch's `fbcon` and the GUI.
   **Gate:** the panel shows a correct image from our kernel.

### Phase 3 - Touch and WiFi
1. Touch: port the tlsc6x driver into the `tpd` framework; verify evdev events
   on `/dev/input/event3`. Sort out any touch firmware/config auto-load.
2. WiFi: build `wlan_drv` + the `wmt` chain from the tree's `connectivity`
   source; load against the firmware blobs and the `nvram`/`nvdata` calibration.
   Verify association and DHCP.
   **Gate:** touch and WiFi work; Couch runs end-to-end on our kernel.

### Phase 4 - The payoff and parity
1. Fix the keypad interrupt handler at the source (the original motivation).
2. Enable the wins the stock kernel denies us: suspend-to-RAM, full ftrace,
   direct CPU governor/hotplug control.
3. Compare against stock for performance and power; fold `build-kernel.sh` into
   Couch's build; document. Add IR last.

## Recovery and safety (non-negotiable)

- **Never write `preloader_*` or `lk`.** They boot the device; a bad one drops
  you to BootROM recovery over USB (mtkclient), not a reboot. Everything here
  flashes only the recovery-slot boot image.
- **The other slot must always boot.** It was Android; it is now the Couch
  recovery image, which by design never shares a kernel with the experiment.
  init arms the BCB so a hung kernel is watchdog-reset into it unattended.
- Keep a known-good `linux-recovery.img` (the stock-kernel Couch build) to
  flash back at any time; it is also where the recovery image takes its kernel.
- The dead-man reboot and `/tmp/stay` claim still apply during bring-up.

## Risk register

| risk | likelihood | mitigation |
|---|---|---|
| GCC-4.9 / Kbuild won't build cleanly | med | Phase 0 gate is exactly this; era-correct toolchain in Docker |
| DTB from our `Image` incompatible | low | reuse the stock appended DTB verbatim in Phase 1 |
| ST7701s init sequence wrong (black panel) | **high** | serial-first bring-up; lift the exact init from `lk.img`; recovery slot |
| WiFi ABI / nvram mismatch | med | build `wlan_drv` from the matching tree; use backup calibration partitions |
| tlsc6x needs a config/firmware blob | med | extract from `vendor.img`/`system.img` in the backup; tlsc6x can auto-load |
| brick | low | never touch preloader/lk; recovery slot only; Android intact |

## Open decisions (resolve in Phase 0)

- **Base tree:** confirm `wiko_k300@alps-3.18.79` builds and is the most complete
  MT6580 3.18.79 base, vs `obraxys/kernel_3.18.79` or `antonialoytorrens`.
- **Sublevel:** stock is 3.18.79; the wiko base is 3.18.79. Keep them matched.
- **DTB source:** stock appended DTB (safe) vs building from the tree's dts.
- **Where WiFi calibration is read from** at runtime (already handled by Couch's
  vendor-blob extraction; confirm it still applies to a rebuilt module).

## Immediate next steps

1. Extract the kernel version string, cmdline and appended DTB from the backup.
2. Build a Docker image with arm-eabi GCC 4.9 and the kernel build deps.
3. Clone `wiko_k300@alps-3.18.79` and get an unmodified build green.

That is Phase 0, and none of it touches the device.
