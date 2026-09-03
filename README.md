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
| WiFi | not yet up — see below |

## Roadmap

1. **WiFi.** Everything needed is already on the device: `/vendor/lib/modules/
   wlan_drv.ko` (+ `wmt_chrdev_wifi.ko`, `wmt_drv.ko`), firmware `WIFI_RAM_CODE_6580`
   and `WMT_SOC.cfg`, and `/vendor/bin/hw/wpa_supplicant`. Unlocks SSH and `apk`.
2. **Alpine rootfs** on the `cache` partition (112MB, expendable).
3. **Move to the `boot` slot** once it is trustworthy.

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
