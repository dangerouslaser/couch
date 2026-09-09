//! Offline, source-attributed IR library. Browsing/importing/saving never emits IR.
use super::{parse, Reply};
use couch_ir::codeset::{self, Codeset, Entry};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path, sync::OnceLock};

fn canonical(entry: &Entry) -> String {
    if let Some(raw) = &entry.raw {
        format!(
            "{} raw {} {}",
            entry.button,
            raw.carrier_hz,
            raw.pattern_us
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    } else {
        format!(
            "{} {} 0x{:x} 0x{:x}",
            entry.button,
            entry.protocol.name(),
            entry.address,
            entry.command
        )
    }
}
fn command(entry: &Entry) -> Value {
    json!({"name":entry.button,"protocol":entry.protocol.name(),"address":entry.address,"command":entry.command,"supported":true,"code":canonical(entry)})
}
fn parsed(text: &str) -> Result<Value, String> {
    let set = Codeset::parse("import", text).map_err(|e| e.to_string())?;
    if set.entries.is_empty() {
        return Err("Choose at least one command".into());
    }
    Ok(
        json!({"commands":set.entries.iter().map(command).collect::<Vec<_>>(),"text":set.entries.iter().map(canonical).collect::<Vec<_>>().join("\n") + "\n", "physically_verified":false}),
    )
}
fn little_endian(text: &str) -> Result<u32, String> {
    let bytes = text
        .split_whitespace()
        .map(|s| {
            u8::from_str_radix(s, 16)
                .map_err(|_| "Expected four hexadecimal little-endian bytes".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let array: [u8; 4] = bytes
        .try_into()
        .map_err(|_| "Expected four hexadecimal little-endian bytes".to_string())?;
    Ok(u32::from_le_bytes(array))
}
fn slug(name: &str) -> String {
    let s = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "_-:+.".contains(c) {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(64)
        .collect::<String>();
    if s.is_empty() {
        "command".into()
    } else {
        s
    }
}
fn flipper(text: &str) -> Result<Value, String> {
    if text.len() > 256 * 1024 {
        return Err("Import exceeds 256 KiB".into());
    }
    let mut header = BTreeMap::new();
    let mut blocks: Vec<BTreeMap<String, String>> = Vec::new();
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once(':').ok_or("Expected Flipper key: value")?;
        if key == "name" {
            if blocks.len() >= 256 {
                return Err("Maximum 256 commands".into());
            }
            blocks.push(BTreeMap::new());
        }
        let dest = if let Some(last) = blocks.last_mut() {
            last
        } else {
            &mut header
        };
        if dest
            .insert(key.trim().to_string(), value.trim().to_string())
            .is_some()
        {
            return Err(format!("Duplicate field {key}"));
        }
    }
    if header.get("Filetype").map(String::as_str) != Some("IR signals file")
        || header.get("Version").map(String::as_str) != Some("1")
    {
        return Err("Expected Flipper IR signals file, Version 1".into());
    }
    if blocks.is_empty() {
        return Err("No commands in file".into());
    }
    let mut commands = Vec::new();
    let mut names = std::collections::HashSet::new();
    let mut lines = Vec::new();
    for block in blocks {
        let get = |key: &str| block.get(key).map(String::as_str).unwrap_or("");
        let original = get("name");
        let base = slug(original);
        let mut name = base.clone();
        let mut n = 2;
        while !names.insert(name.clone()) {
            name = format!("{base}-{n}");
            n += 1;
        }
        let protocol = if get("type") == "raw" {
            "raw"
        } else {
            get("protocol")
        };
        let result = (|| -> Result<String, String> {
            if get("type") == "raw" {
                let duty: f64 = get("duty_cycle")
                    .parse()
                    .map_err(|_| "Missing raw duty cycle")?;
                if !duty.is_finite() || (duty - 0.33).abs() > 0.005 {
                    return Err("The remote blaster supports a 33% duty cycle".into());
                }
                return Ok(format!(
                    "{name} raw {} {}",
                    get("frequency"),
                    get("data").split_whitespace().collect::<Vec<_>>().join(",")
                ));
            }
            if get("type") != "parsed" {
                return Err("Unsupported signal type".into());
            }
            let address = little_endian(get("address"))?;
            let mut command = little_endian(get("command"))?;
            let mapped = match protocol {
                "NEC" => "nec",
                "NECext" => {
                    // Flipper's extended command is 16 bits; Couch's encoder
                    // supplies the inverse byte itself. Never truncate arbitrary data.
                    if command > 65535 || ((command >> 8) as u8) != !(command as u8) {
                        return Err(
                            "NECext command lacks the inverse byte required by this encoder".into(),
                        );
                    }
                    command &= 255;
                    "nec-ext"
                }
                "RC5" => "rc5",
                "RC6" => "rc6",
                "SIRC" => "sony12",
                "SIRC15" => "sony15",
                "SIRC20" => "sony20",
                "Samsung32" => "samsung",
                _ => {
                    return Err(format!(
                        "Protocol {protocol} is not supported; import a raw capture instead"
                    ))
                }
            };
            Ok(format!("{name} {mapped} {address} {command}"))
        })()
        .and_then(|line| {
            Codeset::parse("Flipper command", &line)
                .map_err(|e| e.to_string())
                .map(|set| canonical(&set.entries[0]))
        });
        let mut item = json!({"name":original,"protocol":protocol,"supported":result.is_ok()});
        if let Ok(a) = little_endian(get("address")) {
            item["address"] = json!(a);
        }
        if let Ok(c) = little_endian(get("command")) {
            item["command"] = json!(c);
        }
        match result {
            Ok(line) => {
                item["code"] = json!(line);
                lines.push(line);
            }
            Err(e) => item["reason"] = json!(e),
        }
        commands.push(item);
    }
    Ok(json!({"commands":commands,"text":lines.join("\n") + "\n","physically_verified":false}))
}
fn official_catalog() -> Value {
    serde_json::from_str(include_str!("../../assets/ir/official-index.json"))
        .expect("embedded official IR index")
}
fn detail(item: &Value) -> Result<Value, String> {
    use std::io::Read;
    if let (Some(offset), Some(length)) = (item["offset"].as_u64(), item["length"].as_u64()) {
        let packed = include_bytes!("../../assets/ir/official-data.irpack");
        let end = offset.checked_add(length).ok_or("Invalid catalog offset")?;
        let bytes = packed
            .get(offset as usize..end as usize)
            .ok_or("Invalid catalog offset")?;
        let mut text = String::new();
        flate2::read::GzDecoder::new(bytes)
            .take(256 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        let preview = flipper(&text)?;
        let mut value = item.clone();
        value["text"] = preview["text"].clone();
        value["commands"] = preview["commands"].clone();
        value["physically_verified"] = json!(false);
        value.as_object_mut().unwrap().remove("offset");
        value.as_object_mut().unwrap().remove("length");
        return Ok(value);
    }
    Ok(item.clone())
}

fn catalog() -> &'static Value {
    static CATALOG: OnceLock<Value> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut value: Value = serde_json::from_str(include_str!("../../assets/ir/catalog.json"))
            .expect("embedded IR catalog");
        for item in value["codesets"].as_array_mut().unwrap() {
            let preview =
                flipper(item["text"].as_str().unwrap()).expect("validated catalog source");
            item["text"] = preview["text"].clone();
            item["commands"] = preview["commands"].clone();
            item["physically_verified"] = json!(false);
            item["commands_count"] = json!(item["commands"].as_array().unwrap().len());
            let supported = item["commands"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|c| c["supported"] == true)
                .count();
            item["supported_commands"] = json!(supported);
            item["supported"] = json!(supported > 0);
        }
        let official = official_catalog();
        value["sources"] = json!([value["source"], official["source"]]);
        value["source"] = json!({"name":"Couch IR library · Flipper Devices + Flipper-IRDB", "license":"MIT + CC0-1.0", "url":"https://github.com/flipperdevices/IRDB", "license_text":include_str!("../../assets/ir/LICENSE-Flipper-MIT.txt")});
        value["codesets"].as_array_mut().unwrap().extend(official["codesets"].as_array().unwrap().iter().cloned());
        value
    })
}
fn installed(directory: &Path, id: &str) -> Result<Value, String> {
    let set = codeset::load(directory, id).map_err(|e| e.to_string())?;
    let mut value = parsed(
        &set.entries
            .iter()
            .map(canonical)
            .collect::<Vec<_>>()
            .join("\n"),
    )?;
    value["id"] = json!(id);
    Ok(value)
}
fn save(directory: &Path, id: &str, text: &str) -> Result<Value, String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if !codeset::valid_id(id) {
        return Err("Use a lowercase codeset ID with letters, numbers, hyphens or underscores (maximum 64 characters)".into());
    }
    let mut value = parsed(text)?;
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    if !std::fs::symlink_metadata(directory)
        .map_err(|e| e.to_string())?
        .is_dir()
    {
        return Err("IR directory must not be a symlink".into());
    }
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temporary = directory.join(format!(
        ".save-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let destination = directory.join(format!("{id}.codeset"));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(value["text"].as_str().unwrap().as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, &destination)?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| e.to_string())?;
    value["id"] = json!(id);
    Ok(value)
}
// Bounded process-local toggle state: a new RC5/RC6 press is a new command.
fn test_toggle(directory: &Path, id: &str) -> bool {
    static STATES: OnceLock<
        std::sync::Mutex<std::collections::VecDeque<(std::path::PathBuf, bool)>>,
    > = OnceLock::new();
    let mut states = STATES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let key = directory.join(id);
    let next = states
        .iter()
        .position(|(k, _)| k == &key)
        .and_then(|i| states.remove(i))
        .map(|(_, previous)| !previous)
        .unwrap_or(false);
    if states.len() >= 64 {
        states.pop_front();
    }
    states.push_back((key, next));
    next
}

pub(super) fn route(method: &str, path: &[&str], body: &[u8], directory: &Path) -> Reply {
    let result: Result<Value, String> = match (method, path) {
        ("GET", ["catalog"]) => {
            use std::os::unix::fs::FileTypeExt;
            let full = catalog();
            let items = full["codesets"].as_array().unwrap();
            let summaries = items
                .iter()
                .map(|item| {
                    let mut summary = serde_json::Map::new();
                    for key in [
                        "id",
                        "brand",
                        "device_type",
                        "model",
                        "commands_count",
                        "supported_commands",
                        "supported",
                    ] {
                        summary.insert(key.into(), item[key].clone());
                    }
                    Value::Object(summary)
                })
                .collect::<Vec<_>>();
            let mut value =
                json!({"source":full["source"],"sources":full["sources"],"codesets":summaries});
            for (key, field) in [("brands", "brand"), ("device_types", "device_type")] {
                let unique = items
                    .iter()
                    .filter_map(|v| v[field].as_str())
                    .collect::<std::collections::BTreeSet<_>>();
                value[key] = json!(unique);
            }
            value["blaster_available"] =
                json!(std::fs::metadata("/dev/irtx").is_ok_and(|m| m.file_type().is_char_device()));
            Ok(value)
        }
        ("GET", ["catalog", id]) => match catalog()["codesets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == *id)
        {
            Some(v) => detail(v),
            None => return Reply::error(404, "Codeset not found"),
        },
        ("POST", ["import"]) => {
            let input: Value = match parse(body) {
                Ok(v) => v,
                Err(r) => return r,
            };
            let text = input["text"].as_str().unwrap_or("");
            match input["format"].as_str() {
                Some("flipper") => flipper(text),
                Some("couch") => parsed(text),
                _ => Err("Choose flipper or couch format".into()),
            }
        }
        ("GET", ["codesets"]) => {
            let mut sets = Vec::new();
            if let Ok(entries) = std::fs::read_dir(directory) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    if let Some(id) = name.to_str().and_then(|s| s.strip_suffix(".codeset")) {
                        if let Ok(v) = installed(directory, id) {
                            sets.push(json!({"id":id,"commands_count":v["commands"].as_array().unwrap().len()}));
                        }
                    }
                }
            }
            sets.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
            Ok(json!({"codesets":sets}))
        }
        ("GET", ["codesets", id]) => installed(directory, id),
        ("PUT", ["codesets", id]) => {
            let input: Value = match parse(body) {
                Ok(v) => v,
                Err(r) => return r,
            };
            save(directory, id, input["text"].as_str().unwrap_or(""))
        }
        ("POST", ["codesets", id, "test"]) => {
            let input: Value = match parse(body) {
                Ok(v) => v,
                Err(r) => return r,
            };
            (|| {
                let set = codeset::load(directory, id).map_err(|e| e.to_string())?;
                let entry = set
                    .get(input["command"].as_str().unwrap_or(""))
                    .ok_or("Choose an installed command")?;
                let message = entry
                    .encode(test_toggle(directory, id))
                    .map_err(|e| e.to_string())?;
                let mut blaster =
                    couch_ir::tx::Irtx::open("/dev/irtx").map_err(|e| e.to_string())?;
                couch_ir::tx::transmit(&mut blaster, &message, 0).map_err(|e| e.to_string())?;
                Ok(json!({"sent":true,"physically_verified":false}))
            })()
        }
        _ => return Reply::error(404, "Not found"),
    };
    match result {
        Ok(v) => Reply::json(200, &v),
        Err(e) => Reply::error(400, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn signal(protocol: &str, address: &str, command: &str) -> String {
        format!("Filetype: IR signals file\nVersion: 1\n#\nname: Power\ntype: parsed\nprotocol: {protocol}\naddress: {address}\ncommand: {command}\n")
    }
    #[test]
    fn test_presses_alternate_the_rc_toggle_per_codeset() {
        let p = Path::new("/isolated-test-toggle");
        assert!(!test_toggle(p, "one"));
        assert!(test_toggle(p, "one"));
        assert!(!test_toggle(p, "two"));
        assert!(!test_toggle(p, "one"));
    }
    #[test]
    fn flipper_little_endian_and_necext_inverse_are_preserved() {
        let preview = flipper(&signal("NECext", "34 12 00 00", "08 F7 00 00")).unwrap();
        assert_eq!(preview["text"], "power nec-ext 0x1234 0x8\n");
        let bad = flipper(&signal("NECext", "34 12 00 00", "08 00 00 00")).unwrap();
        assert_eq!(bad["commands"][0]["supported"], false);
        assert!(bad["commands"][0]["code"].is_null());
        assert_eq!(little_endian("01 02 03 04").unwrap(), 0x04030201);
        assert!(little_endian("01").is_err());
    }
    #[test]
    fn unsupported_protocol_and_invalid_ranges_stay_visible() {
        for (protocol, address, command) in [
            ("Kaseikyo", "00 00 00 00", "01 00 00 00"),
            ("NEC", "00 01 00 00", "01 00 00 00"),
        ] {
            let v = flipper(&signal(protocol, address, command)).unwrap();
            assert_eq!(v["commands"][0]["supported"], false);
            assert!(v["commands"][0]["reason"].as_str().unwrap().len() > 5);
        }
    }
    #[test]
    fn raw_signal_requires_supported_carrier_duty_and_bounded_timings() {
        let raw="Filetype: IR signals file\nVersion: 1\nname: Power\ntype: raw\nfrequency: 38000\nduty_cycle: 0.330000\ndata: 9000 4500 560 560\n";
        let v = flipper(raw).unwrap();
        assert_eq!(v["text"], "power raw 38000 9000,4500,560,560\n");
        for bad in [
            raw.replace("0.330000", "0.500000"),
            raw.replace("38000", "1000000"),
            raw.replace("9000 4500", "4294967295 4500"),
        ] {
            assert_eq!(flipper(&bad).unwrap()["commands"][0]["supported"], false);
        }
    }
    #[test]
    fn imports_reject_bad_headers_and_duplicate_fields_and_disambiguate_names() {
        assert!(flipper("Filetype: IR signals file\nVersion: 2").is_err());
        let source = signal("NEC", "04 00 00 00", "08 00 00 00");
        assert!(flipper(&(source.clone() + "command: 08 00 00 00\n")).is_err());
        let repeated = source.clone() + source.split("#\n").nth(1).unwrap();
        let v = flipper(&repeated).unwrap();
        assert!(v["text"].as_str().unwrap().contains("power-2 nec"));
    }
    #[test]
    fn all_bundled_sources_parse_with_unique_ids_and_verified_hash_fields() {
        let value = catalog();
        let items = value["codesets"].as_array().unwrap();
        assert!(items.len() >= 40);
        let mut ids = std::collections::HashSet::new();
        for item in items {
            assert!(ids.insert(item["id"].as_str().unwrap()));
            assert!(["CC0-1.0", "MIT"].contains(&item["license"].as_str().unwrap()));
            let detailed = detail(item).unwrap_or_else(|e| panic!("{}: {e}", item["path"]));
            assert_eq!(
                detailed["commands"].as_array().unwrap().len() as u64,
                item["commands_count"].as_u64().unwrap(),
                "{}",
                item["path"]
            );
            assert_eq!(
                detailed["commands"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|c| c["supported"] == true)
                    .count() as u64,
                item["supported_commands"].as_u64().unwrap(),
                "{}",
                item["path"]
            );
            assert_eq!(detailed["physically_verified"], false);
            assert_eq!(item["sha256"].as_str().unwrap().len(), 64);
            if item["supported"] == true {
                parsed(detailed["text"].as_str().unwrap()).unwrap();
            }
        }
    }
    #[test]
    fn saved_codesets_round_trip_atomically_without_traversal_or_symlink_reads() {
        let directory = std::env::temp_dir().join(format!("couch-ir-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let v = save(&directory, "user-tv", "power nec 4 8\n").unwrap();
        assert_eq!(installed(&directory, "user-tv").unwrap(), v);
        assert!(save(&directory, "../escape", "power nec 4 8").is_err());
        assert!(save(&directory, "empty", "").is_err());
        assert!(save(&directory, "duplicate", "power nec 4 8\nPower nec 4 9").is_err());
        std::os::unix::fs::symlink(
            directory.join("user-tv.codeset"),
            directory.join("link.codeset"),
        )
        .unwrap();
        assert!(installed(&directory, "link").is_err());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
