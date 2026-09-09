#!/bin/busybox sh
# Print the boot setup policy. Arguments: saved-network count, recovery/no-UI,
# explicit hotspot request. Connection failures never imply a factory reset.
if [ "${3:-0}" = 1 ]; then
    echo recovery
elif [ "${2:-0}" != 1 ] && [ "${1:-0}" -eq 0 ]; then
    echo local
else
    echo normal
fi
