# Couch

A small Linux distribution for the couch: it runs on a Home Assistant remote
control, the Sanytron Astrion HA100.

```
couch:~# uname -a
Linux couch 3.18.79 #7 SMP PREEMPT armv7l GNU/Linux
```

Alpine userland, WiFi, SSH, a framebuffer console, and a setup portal for
configuring it without a cable.

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

## WiFi

Works. `tools/provision-alpine.sh` bakes `wpa_supplicant` and its dependencies
into the base rootfs, because without them there is no network, and without a
network `apk` cannot fetch them.

Five things had to be right, and four of them fail silently:

1. **Do not restore Android's property area before running `wmt_loader`.**
   It reads `persist.mtk.wcn.combo.chipid`; if that is already set it assumes
   detection has happened and exits without ever opening `/dev/wmtdetect`. The
   snapshot that was meant to help WiFi was the thing preventing it. With no
   properties it reads the id from hardware (`ioctl` returns `0x6580`) and hands
   it to the driver. Restore the properties afterwards.
2. **`stpwmt` (major 190) is registered only after successful detection**, so
   waiting for the node before running the loader waits for something only the
   loader can produce. There is no devtmpfs, so `mknod` it afterwards.
3. **Use Alpine's `wpa_supplicant`, not `/vendor/bin/hw/wpa_supplicant`.** The
   vendor build logs solely to Android's logd and exits silently with no output
   at all when logd is not running - `-f` is not compiled in either.
4. **Emit every stored network, not just the first.** The first entry in
   Android's `WifiConfigStore.xml` is not necessarily the one in range.
5. **Run `udhcpc` inside the chroot.** busybox udhcpc does nothing without
   `/usr/share/udhcpc/default.script`; from the initramfs it takes the lease and
   never applies it, which looks exactly like DHCP failing.

Credentials are read from Android's own store on the device at runtime, so they
never live in this repo or the boot image. userdata is **f2fs**, not ext4.

Writing `/dev/wmtWifi` while the chip is half-initialised panics the kernel in
`wmt_drv` (`__list_add` on an uninitialised mutex) and hard-resets the device.
Get the order right and it is fine.

## Setup portal

With no known network, the remote hosts its own. `stage2` falls back to
`portal.sh`, which brings up an AP, a wildcard-DNS captive portal and a small web
UI for choosing a network and enrolling an SSH key.

It is deliberately not an open AP. The AP runs **WPA2 with a random passphrase
generated per boot and shown on the device's own screen**, so joining it requires
physically looking at the remote. That is what makes enrolling an SSH key through
a web page acceptable: an open setup network would let anyone in radio range take
permanent root, and the exposure would not be brief - a device whose WiFi password
changed would sit in setup mode indefinitely.

On top of that: enrolling a key needs a **physical button press** on the remote
(read from evdev), the portal **times out** after ten minutes, and the shipped
image **contains no authorised keys at all**, so a fresh device trusts nobody.

Because a radio cannot scan while in AP mode, the network list is captured just
before switching and cached for the portal to serve.

## Reading the vendor source

`pwr_on fail(-3)` is not a mystery once you have the source. MediaTek's MT6580
kernel tree is public ([Mysteryagr/MT6580-Kernel-3.18](https://github.com/Mysteryagr/MT6580-Kernel-3.18),
180MB) and contains the exact driver our binary module was built from -
`connectivity/wlan/gen2/common/{wlan_lib,wlan_oid,wlan_p2p,wlan_bow}.c` matches
the paths embedded in `wlan_drv.ko`, and `connectivity/common/conn_soc` is the
SoC-connectivity variant the vendor ships as `common_main`.

In `conn_soc/core/wmt_core.c`, the ops lookup is a switch on the chip id:

```c
    case 0x6580:
        p_ops = &wmt_ic_ops_soc;
        break;
    default:
        p_ops = (P_WMT_IC_OPS) NULL;
    }
    if (NULL == p_ops) {
        WMT_ERR_FUNC("unsupported chip id (hw_code): 0x%x\n", chipid);
        return -3;
    }
```

So **-3 means "unsupported chip id"**: 0x6580 *is* supported, and the failure is
that the driver had no valid chip id when power-on ran. That is also the earlier
`gMtkWmtCtx.p_ic_ops is NULL` line - the same cause seen from the other end.

`wmt_loader` hands the id over by ioctl on `/dev/wmtdetect`; a working run shows

```
ioctl(_IOC_READ,  0x77, 0x3) = 25984     # 0x6580, read from hardware
ioctl(_IOC_WRITE, 0x77, 0x1, 0x6580) = 0 # handed to the driver
```

Anything that stops the loader reaching those ioctls leaves the driver with no
chip id and power-on fails with -3 - which is why restoring Android's property
area before the loader breaks WiFi (see above): the loader sees
`persist.mtk.wcn.combo.chipid` already set, assumes detection happened, and
exits without ever opening `/dev/wmtdetect`.

Having the source also makes a **rebuild** attractive: the tree includes four
MT6580 defconfigs, and this device's exact config is in the backup as
`config-stock.txt` (3867 lines). Rebuilding the vendor 3.18 kernel with
`CONFIG_VT` and `CONFIG_DEVTMPFS` enabled would retire fbcon and the hand-made
device nodes, while keeping the WiFi driver that only exists downstream. It
needs a gcc-4.9-era ARM cross toolchain, which Docker can supply.

### Why the rebuild is blocked

Rebuilding from public source does not work for this board, and it is worth
knowing before spending a day on a toolchain. Three drivers this device needs are
absent from every public MT6580 tree checked
([Mysteryagr](https://github.com/Mysteryagr/MT6580-Kernel-3.18),
[parthibx24/k80](https://github.com/parthibx24/android_kernel_mediatek_k80),
[LCM-MTK](https://github.com/LCM-MTK/android_kernel_mediatek_mt6580)):

| needed | status |
| --- | --- |
| `CONFIG_MTK_IRTX_PWM_SUPPORT` (IR transmit) | `irtx/` has only Kconfig and Makefile in all three trees; the Makefile pulls `irtx/$(CONFIG_MTK_PLATFORM)/`, and `irtx/mt6580/` does not exist |
| `st7701s_wvga_dsi_vdo_boe_tn_tianxian` (panel) | absent; 42 other panels present |
| `tlsc6x` (touchscreen) | absent; ektf2k / ft5x0x / ft6336 present |

IR is compiled in rather than built as a module, so it cannot be lifted out of
the stock kernel either. A kernel built from these trees would boot with no
display, no touch and no IR - which is the entire device.

The unblock is a GPL source request. A kernel panic leaked the vendor's build
path, `/home/felix/ha100fw/alps/out/target/product/x15cm_s90_kr/...`, so
Sanytron build from a full MediaTek ALPS tree and distribute a GPL-2.0 kernel;
they are obliged to provide its source, which would be the exact tree including
all three drivers. Ask for the kernel source corresponding to the shipped build
(`3.18.79 #7`, `RS30_HAOS_HA100_V1.0.4`).

Mainline is a different road: [u-boot-mt6580](https://github.com/predefine-mt6580/u-boot-mt6580)
has active MT6580 work (clocks, eMMC, display PWM), but `mt76` does not cover
MT6580's in-SoC CONSYS, so a mainline kernel means no WiFi at all.

## Distribution

The image is meant to be reusable, so it carries nothing device- or
owner-specific:

* **no SSH keys** - enrolment happens through the portal or over USB
* **no WiFi credentials** - read from Android's store on the device at runtime,
  or entered through the portal and saved to the Alpine partition
* **no Android property snapshot** - it contained this unit's serial number, and
  it broke WiFi detection anyway

`root` is unlocked with `*` rather than `!` in `/etc/shadow`: sshd refuses to
complete authentication for a locked account even with a valid key, but `*`
leaves it with no password at all, so key auth works and password auth cannot.

## Roadmap

1. ~~Alpine rootfs~~ — done.
2. ~~WiFi~~ — done.
3. ~~Captive portal~~ — done.
4. **Move to the `boot` slot**, putting Android's boot image into `recovery` so
   lk's menu still reaches it. Tooling exists (`tools/swap-slots.sh`), not applied.

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
