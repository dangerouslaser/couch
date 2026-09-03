# Linux on the Sanytron Astrion HA100

Real Linux, running on a Home Assistant remote control.

```
Linux (none) 3.18.79 #7 SMP PREEMPT armv7l GNU/Linux
uid=0 gid=0
```

The HA100 is a MediaTek MT6580 (quad Cortex-A7, ARMv7, 1GB RAM, 480x800 touchscreen)
shipping Android 8.1. This boots a non-Android userland on it and gives you a root
shell over the USB cable.

## The approach, and why

Mainline Linux knows about the MT6580 — `arch/arm/boot/dts/mediatek/mt6580.dtsi` is
in Torvalds' tree — but it is a 108-line skeleton: four CPUs, GIC, a timer, two UARTs,
and *dummy* fixed clocks. No clock controller, no pinctrl, no MMC, no display. Every
peripheral that matters here is out-of-tree vendor code: the display is `mtkfb`, WiFi
is the in-SoC `CONSYS_MT6735` block, and the IR blaster is a `mt_irtx` char driver at
major 243. So this uses the **stock 3.18.79 vendor kernel, byte-for-byte**, and
replaces only the ramdisk.

It boots from the **recovery** slot, never `boot`. Android stays completely intact and
bootable, which makes every experiment a power-cycle away from a working remote.

Nothing is signed. The stock boot image has 8.7MB of trailing zeros where a signature
block would be, so `lk` boots unsigned images and a plain `dd` from rooted Android is
enough — no bootloader unlock, no fastboot, no BootROM exploit.

## Usage

```sh
tools/backup.sh    # FIRST. dumps all 23 partitions (~1.7GB) outside the repo
tools/build.sh     # stock kernel + initramfs -> build/linux-recovery.img
tools/flash.sh     # writes it to the recovery slot
tools/boot.sh      # reboots into Linux and claims the serial shell
tools/markers.sh   # offline debug: progress markers + kernel logs
python3 tools/sercmd.py 'uname -a'    # run a command on the device
```

## Getting back to Android

* init clears the 512-byte BCB in `para` as its first action, so **the next reboot
  returns to Android by itself**. Only the first 512 bytes are cleared — an `ENV_v1`
  block lives at offset 128K and must survive.
* Unclaimed sessions self-reboot after 4 minutes. `touch /tmp/stay` keeps one alive.
* If a kernel panics before init runs, the recovery flag persists and it will retry.
  Escape via lk's boot menu: hold Volume Up at power-on, **Volume Up moves the
  selector, Volume Down selects**, choose `[Normal Boot]`.

## Getting anything onto the screen

There is no `CONFIG_VT`, so no kernel fbcon and no console. `src/fbcon.c` draws
init's stdout with the kernel's own 8x16 font. Four separate bugs each produce an
identical blank screen, so fixing them one at a time looks like no progress:

1. **The panel switches its own backlight off when idle.** An early build blinked
   the backlight as a heartbeat and was accidentally keeping the screen alive.
   init now re-asserts brightness every 5s, and fbcon does after every frame.
2. **Alpha.** mtkfb composites ARGB8888. Colours built as `0x00RRGGBB` have alpha
   0, so they are perfectly correct in memory, readable back from `/dev/fb0`, and
   completely transparent on the glass.
3. **Write size.** mtkfb transfers to the panel per `write()`. A single 1.5MB
   write lands in memory and never reaches the display; 4096-byte chunks do.
   This is why plain `dd` displayed all along and fbcon never did.
4. **The font.** Extracting glyphs from `lib/fonts/font_8x16.c` with a naive
   `0x..` regex also matches the hex inside each per-glyph comment
   (`/* 65 0x41 'A' */`), shifting every glyph by one byte. Strip comments first,
   then assert that space is blank before writing the header.

One more trap: `fb_var_screeninfo` reports `red=0/8 green=8/8 blue=16/8`, implying
ABGR. It is actually ARGB - trust it and blue renders as orange.

`tools/sercmd.py 'cat /tmp/fbcon.geom'` dumps what the driver reported.

## Four things that will waste your day

1. **No `CONFIG_VT`.** There is no virtual terminal, so `exec >/dev/console` fails and
   takes init with it — the kernel panics with no output at all. Never redirect to
   the console. It also means a successful boot looks identical to a hung one: lk's
   logo simply stays on screen because nothing overwrites the framebuffer.
2. **No `CONFIG_DEVTMPFS`.** Nothing populates `/dev`. Device nodes are baked into the
   cpio by `tools/mkcpio.py`; `mdev -s` fills in the rest.
3. **`f_acm/instances` defaults to 0.** Setting `functions=acm` then gives a gadget
   advertising a configuration with no endpoints. The host sits at `CONNECTED` and
   never reaches `CONFIGURED`, and the kernel logs `cfg 1 speeds:` with nothing after
   it. Write `1` to `instances` *before* `functions`.
4. **The MediaTek preloader exposes its own CDC port ~6s into boot**, which is easy to
   mistake for your gadget. It disappears seconds later. `tools/boot.sh` waits 25s.

macOS has no RNDIS or EEM driver, so CDC-ACM is the only usable gadget of the ones
this kernel offers.

## What works

| | |
|---|---|
| RAM | 937MB of 979MB free (Android leaves far less) |
| CPU | 4 cores, after onlining `cpu1-3` (only cpu0 comes up by default) |
| Display | `mtkfb`, 480x800x32 at `/dev/fb0`, writable at 13MB/s |
| Input | `mt_gpio_kpd`, `mtk-kpd` (buttons), `mtk-tpd` (touchscreen) |
| IR | `/dev/irtx`, `mt_irtx` driver loaded |
| Watchdog | kicked by kernel threads `wdtk-0..3`; userspace need not |
| WiFi | **parked** — see below |
| Userland | Alpine 3.21.7 armv7 on the cache partition, with working `apk` |

## The Alpine userland

Alpine armv7 lives on the `cache` partition and is installed *from Android* over adb,
which is far faster than pushing it down a serial line and needs no network at all:

```sh
adb shell 'umount /cache; mke2fs -t ext4 -b 4096 -L alpine -F /dev/block/mmcblk0p22'
adb shell 'mkdir -p /data/local/tmp/alp && mount -t ext4 /dev/block/mmcblk0p22 /data/local/tmp/alp'
adb push alpine-minirootfs-3.21.7-armv7.tar.gz /data/local/tmp/alpine.tgz
adb shell 'cd /data/local/tmp/alp && /data/local/tmp/busybox tar xzf /data/local/tmp/alpine.tgz'
```

Use busybox's `tar`, not Android's toybox, so symlinks and permissions survive.

init chroots into it rather than `switch_root`-ing into OpenRC, because OpenRC expects
a devtmpfs this kernel does not have. The initramfs stays PID 1 and supervises.

## WiFi: why it is parked

Everything needed is on the device — `/vendor/lib/modules/wlan_drv.ko`,
`wmt_chrdev_wifi.ko`, `wmt_drv.ko`, firmware `WIFI_RAM_CODE_6580`, and
`/vendor/bin/hw/wpa_supplicant` — but the bring-up chain fights back:

* `wmt_loader` talks to `/dev/stpwmt`, which `wmt_drv` registers asynchronously.
  Run it too early and it exits 2 with an empty log, which is easy to misread as
  the chip being absent.
* It also reads Android properties (`persist.mtk.wcn.combo.chipid` = `0x6580`).
  With no property area every read returns empty. Restoring a snapshot of
  `/dev/__properties__` fixes that — and is useful for every other vendor binary.
* Once it gets *partway*, writing `/dev/wmtWifi` panics the kernel:
  `osal_lock_sleepable_lock` -> `__list_add` on a mutex whose list head is NULL,
  then `emergency_restart`. The device hard-resets.

`WIFI=1` at the top of `initramfs/init` re-enables the attempt. It is guarded: it
checks the loader's exit code and greps dmesg for `pwr_on fail` / `p_ic_ops is NULL`
and refuses rather than panicking.

## Roadmap

1. ~~Alpine rootfs~~ — done.
2. **Move to the `boot` slot**, putting Android's boot image into `recovery` so lk's
   menu still reaches it.
3. WiFi, when the vendor chain is worth another round.

The remote's existing control app is Android/Kotlin, so replacing Android outright
means rewriting that UI against the raw framebuffer and evdev.

## Layout

```
initramfs/init      what PID 1 does: markers, USB gadget, shell, heartbeat
tools/bootimg.py    unpack/pack MediaTek boot images (header v0, 2048B pages)
tools/mkcpio.py     newc cpio builder with real device nodes and root ownership
tools/sercmd.py     run a command over the USB serial shell
```

The partition backup lives outside the repo and is gitignored: it is 1.7GB and
contains per-unit calibration that should not be committed or shared.
