#!/bin/sh
# The config UI on this machine, against a throwaway house.
#
# --www rather than the copy baked into the binary, so editing style.css and
# running trunk is the whole loop: rebuilding the daemon to see a colour change
# is the wrong shape of wait. The config defaults to a scratch file for the
# same reason - this is for looking at the UI, not for editing anybody's house.
#
# The daemon serves the API and the page from one origin, so there is no CORS
# and no second port. `trunk serve` is the other loop (it proxies /api here);
# see web/couch-web/Trunk.toml.
set -e
cd "$(dirname "$0")/.."

ADDR=${COUCH_CONFD_ADDR:-127.0.0.1:8090}
CONFIG=${COUCH_CONFIG:-build/couch-config.json}
mkdir -p "$(dirname "$CONFIG")"

[ -d web/couch-web/dist ] || {
    echo "no web/couch-web/dist - run tools/build-webui.sh --host first"; exit 1; }
( cd daemon && cargo build --release )

echo "= http://$ADDR   config $CONFIG"
exec daemon/target/release/couch-confd \
    --addr "$ADDR" --config "$CONFIG" --www web/couch-web/dist
