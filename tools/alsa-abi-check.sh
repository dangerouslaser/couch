#!/bin/sh
# Prove that clients/couch-voice/src/abi.rs matches the kernel's real ALSA ABI.
#
# That file transcribes five structs and sixteen ioctl numbers out of
# include/uapi/sound/asound.h by hand, because linking alsa-lib would need a
# cross sysroot this project deliberately does not have. A transcription error
# there does not fail to compile - it produces ENOTTY or EFAULT from a driver
# that is working perfectly, which is the most expensive kind of wrong.
#
# So the numbers are checked against the header itself, compiled as C
# _Static_asserts for armv7a-linux-androideabi. That is the same 32-bit ARM
# EABI the device's musl uses - long is 4 bytes, long long is 8-byte aligned -
# so the layouts are the ones the kernel will see. The device runs 3.18, whose
# asound.h defines all five of these structs identically to the NDK's copy;
# the one struct that did change across versions, snd_pcm_status, is
# deliberately not used by this crate.
#
# Nothing is executed: if it compiles, the assertions hold.
set -e
cd "$(dirname "$0")/.."

NDK="${NDK:-$HOME/Library/Android/sdk/ndk/29.0.14206865}"
CC="$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin/armv7a-linux-androideabi21-clang"
[ -x "$CC" ] || { echo "no NDK clang at $CC (set NDK=...)"; exit 1; }

OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

cat > "$OUT/check.c" <<'CEOF'
#include <sys/ioctl.h>
#include <sound/asound.h>
#include <stddef.h>

#define A(c) _Static_assert(c, #c)

/* struct sizes and the offsets abi.rs depends on */
A(sizeof(struct snd_pcm_info) == 288);
A(offsetof(struct snd_pcm_info, id) == 16);
A(offsetof(struct snd_pcm_info, name) == 80);
A(offsetof(struct snd_pcm_info, subname) == 160);
A(offsetof(struct snd_pcm_info, dev_class) == 192);
A(offsetof(struct snd_pcm_info, subdevices_count) == 200);

A(sizeof(struct snd_mask) == 32);
A(sizeof(struct snd_interval) == 12);
A(sizeof(struct snd_pcm_hw_params) == 604);
A(offsetof(struct snd_pcm_hw_params, masks) == 4);
A(offsetof(struct snd_pcm_hw_params, intervals) == 260);
A(offsetof(struct snd_pcm_hw_params, rmask) == 512);
A(offsetof(struct snd_pcm_hw_params, cmask) == 516);
A(offsetof(struct snd_pcm_hw_params, info) == 520);
A(offsetof(struct snd_pcm_hw_params, fifo_size) == 536);

A(sizeof(struct snd_pcm_sw_params) == 104);
A(offsetof(struct snd_pcm_sw_params, avail_min) == 12);
A(offsetof(struct snd_pcm_sw_params, boundary) == 36);
A(offsetof(struct snd_pcm_sw_params, proto) == 40);
A(offsetof(struct snd_pcm_sw_params, tstamp_type) == 44);

A(sizeof(struct snd_xferi) == 12);

A(sizeof(struct snd_ctl_card_info) == 376);
A(sizeof(struct snd_ctl_elem_id) == 64);
A(offsetof(struct snd_ctl_elem_id, name) == 16);
A(offsetof(struct snd_ctl_elem_id, index) == 60);
A(sizeof(struct snd_ctl_elem_list) == 72);
A(offsetof(struct snd_ctl_elem_list, pids) == 16);
A(sizeof(struct snd_ctl_elem_info) == 272);
A(offsetof(struct snd_ctl_elem_info, type) == 64);
A(offsetof(struct snd_ctl_elem_info, access) == 68);
A(offsetof(struct snd_ctl_elem_info, count) == 72);
A(offsetof(struct snd_ctl_elem_info, owner) == 76);
A(offsetof(struct snd_ctl_elem_info, value) == 80);
A(sizeof(struct snd_ctl_elem_value) == 712);
A(offsetof(struct snd_ctl_elem_value, value) == 72);

/* the ioctl numbers, as the kernel computes them for this ABI */
A((unsigned)SNDRV_PCM_IOCTL_PVERSION     == 0x80044100u);
A((unsigned)SNDRV_PCM_IOCTL_INFO         == 0x81204101u);
A((unsigned)SNDRV_PCM_IOCTL_HW_REFINE    == 0xC25C4110u);
A((unsigned)SNDRV_PCM_IOCTL_HW_PARAMS    == 0xC25C4111u);
A((unsigned)SNDRV_PCM_IOCTL_HW_FREE      == 0x00004112u);
A((unsigned)SNDRV_PCM_IOCTL_SW_PARAMS    == 0xC0684113u);
A((unsigned)SNDRV_PCM_IOCTL_PREPARE      == 0x00004140u);
A((unsigned)SNDRV_PCM_IOCTL_RESET        == 0x00004141u);
A((unsigned)SNDRV_PCM_IOCTL_START        == 0x00004142u);
A((unsigned)SNDRV_PCM_IOCTL_DROP         == 0x00004143u);
A((unsigned)SNDRV_PCM_IOCTL_READI_FRAMES == 0x800C4151u);
A((unsigned)SNDRV_CTL_IOCTL_PVERSION        == 0x80045500u);
A((unsigned)SNDRV_CTL_IOCTL_CARD_INFO       == 0x81785501u);
A((unsigned)SNDRV_CTL_IOCTL_ELEM_LIST       == 0xC0485510u);
A((unsigned)SNDRV_CTL_IOCTL_ELEM_INFO       == 0xC1105511u);
A((unsigned)SNDRV_CTL_IOCTL_ELEM_READ       == 0xC2C85512u);
A((unsigned)SNDRV_CTL_IOCTL_ELEM_WRITE      == 0xC2C85513u);
A((unsigned)SNDRV_CTL_IOCTL_PCM_NEXT_DEVICE == 0x80045530u);
A((unsigned)SNDRV_CTL_IOCTL_PCM_INFO        == 0xC1205531u);
CEOF

"$CC" -c "$OUT/check.c" -o "$OUT/check.o"
echo "abi.rs agrees with sound/asound.h for armv7a-linux-androideabi (32-bit ARM EABI)"
