# On-device setup

Normal first boot opens **Welcome to couch.** on the remote. Choose **Set up
Wi-Fi**, select a scanned network (or enter a hidden SSID), and use the keyboard
to enter its password. **Test connection** checks association and DHCP; it does
not require internet access. **Save network** persists only the tested network.
The existing 60-second trial timeout restores the previous connection if Save
is not selected.

After saving, the remote shows a QR code and its `http://<address>:8090` URL.
Open it from the same network, pair using the PIN displayed on the remote, then
set the timezone/12- or 24-hour clock under Remote settings and add connections,
rooms, and activities. Clock configuration currently happens in the webUI.
**Set up later** allows offline use; a device with no saved networks offers setup
again on its next boot.

## Outages and recovery

Saved networks suppress first-boot onboarding even when association or DHCP
fails. The station supplicant remains running, and the normal remote interface
stays available. Use Settings → Wi-Fi → Change network to repair a connection.
A transient outage never automatically exposes a hotspot.

Settings → Wi-Fi → Recovery hotspot (also on the welcome screen) explicitly
starts the existing `Couch-Setup` portal. This switches the radio away from its
normal connection. Restart the remote to return to normal station mode. The
portal retains its existing physical approval requirement for SSH enrollment.
Headless rescue can explicitly request it with `COUCH_SETUP_AP=1`; it does not
start the graphical onboarding flow.

## Implementation and validation

`stage2/setup-mode.sh` chooses normal/local/recovery boot behavior.
`stage2/setup-watch.sh` handles local recovery requests and starts already
enrolled SSH access after a successful Wi-Fi save. Both are provisioned with
the other `stage2/*.sh` files. No device credentials belong in an image or Git.

Run `python3 -m unittest discover -s tools/tests` for boot policy regressions
and `cargo test --manifest-path ui/Cargo.toml` for network trial/rollback tests.
Validate scanning, keyboard navigation, errors, saving, and the QR handoff on
hardware separately. Do not erase an existing user's networks to test first boot.
