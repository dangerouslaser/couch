#!/bin/busybox sh
# Which paths do the MTK connectivity binaries expect? Anything absent from the
# bundle is a candidate for why the chip will not power up.
BB=/bin/busybox
for b in /vendor/bin/wmt_launcher /vendor/bin/wmt_loader /vendor/lib/modules/wmt_drv.ko; do
    echo "=== $b"
    $BB strings "$b" 2>/dev/null | $BB grep -E '^/(vendor|system|etc|data|proc|dev|sys)/[A-Za-z0-9_./-]+$' \
        | $BB sort -u | while read p; do
            [ -e "$p" ] && echo "  ok      $p" || echo "  MISSING $p"
        done | $BB grep MISSING | $BB head -14
done
