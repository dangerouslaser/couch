#!/bin/sh
# Remove regenerable host debug caches, retaining release/device images and tools.
set -eu
cd "$(dirname "$0")/.."
mode=${1:---dry-run}
case "$mode" in --dry-run|--apply) ;; *) echo "Usage: $0 [--dry-run|--apply]" >&2; exit 2;; esac
if [ "$mode" = --apply ] && pgrep -x 'cargo|rustc' >/dev/null 2>&1; then
    echo 'A Rust build is running; retry cleanup after it finishes.' >&2
    exit 1
fi
# Keep preview caches while iterating on the site; its initial build is expensive.
# Never traverse build/: it contains private device backups and recovery images.
for workspace in ui clients daemon model web; do
    [ -f "$workspace/Cargo.toml" ] || continue
    for cache in deps build incremental .fingerprint examples; do
        path="$workspace/target/debug/$cache"
        [ -d "$path" ] || continue
        [ ! -L "$workspace/target" ] && [ ! -L "$workspace/target/debug" ] && [ ! -L "$path" ] || {
            echo "Skipping symlinked cache: $path" >&2
            continue
        }
        du -sh "$path"
        if [ "$mode" = --apply ]; then rm -rf -- "$path"; fi
    done
done
if [ "$mode" = --dry-run ]; then echo 'Preview only. Use --apply to remove these regenerable caches.'; fi
