/// Use the same tzdata files as the GUI, including Alpine when launched by stage2.
pub(super) fn timezones() -> Vec<String> {
    let mut zones = vec!["UTC".to_string()];
    for root in [
        "/usr/share/zoneinfo",
        "/mnt/alpine/usr/share/zoneinfo",
        "/var/db/timezone/zoneinfo",
    ] {
        if let Ok(text) = std::fs::read_to_string(format!("{root}/zone.tab")) {
            zones.extend(
                text.lines()
                    .filter(|l| !l.starts_with('#'))
                    .filter_map(|l| l.split_whitespace().nth(2))
                    .map(str::to_string),
            );
        }
    }
    zones.sort();
    zones.dedup();
    zones
}
