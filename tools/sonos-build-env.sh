# Sourced by the build scripts from the repository root, before cargo runs.
#
# couch-sonos compiles COUCH_SONOS_BUILT_IN_API_KEY into every binary that
# links it (the CLI, the daemon, the GUI), so a shipped remote identifies itself
# to Sonos players with the project's developer key and no file on the device.
# The repository is public: the key lives in the gitignored build/ directory on
# the build machine, never in the tree. An absent file is not an error - the
# build falls back to the placeholder key, which players accept today.
if [ -z "${COUCH_SONOS_BUILT_IN_API_KEY:-}" ] && [ -f build/sonos-api-key ]; then
    COUCH_SONOS_BUILT_IN_API_KEY=$(tr -d '[:space:]' < build/sonos-api-key)
    export COUCH_SONOS_BUILT_IN_API_KEY
fi
if [ -n "${COUCH_SONOS_BUILT_IN_API_KEY:-}" ]; then
    echo '= Sonos developer key: compiled in' >&2
else
    echo '= Sonos developer key: none (no build/sonos-api-key); using the placeholder' >&2
fi
