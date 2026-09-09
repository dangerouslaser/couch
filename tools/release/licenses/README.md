# Lato notice provenance

`Lato-OFL.txt` is the upstream Google Fonts notice from
https://raw.githubusercontent.com/google/fonts/main/ofl/lato/OFL.txt,
retrieved September 9, 2026. It is copied unchanged into clean Couch payloads.

The existing GUI fonts were compared byte-for-byte with the corresponding
`Lato-Regular.ttf` and `Lato-Bold.ttf` from that same upstream directory:

- Regular SHA-256: `d636e4683231f931eda222d588e944d082bfd3bdba02f928bee461c0f185b251`
- Bold SHA-256: `8a0aace75d33794eece4b28187bfc1df0bbd2888b5d8a56e01788c8d65d16be1`

Both matched. The GUI embeds rasterized font resources; retaining a readable
notice in `/opt/couch/licenses` makes its attribution available in the image.
Inter and Lucide notices are already tracked under `assets/` and are included
separately. This notice inventory does not approve the remaining project,
Slint/Rust dependency, kernel/BusyBox or vendor redistribution requirements.
