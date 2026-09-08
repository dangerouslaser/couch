//! Resolve input devices by identity; event numbers change with probe order.
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

fn named_nodes(sys: &Path, dev: &Path, names: &[&str]) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(sys) else {
        return Vec::new();
    };
    let mut nodes = Vec::new();
    for entry in entries.flatten() {
        let event = entry.file_name();
        let name = event.to_string_lossy();
        let Some(number) = name.strip_prefix("event") else {
            continue;
        };
        if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(identity) = fs::read_to_string(entry.path().join("device/name")) else {
            continue;
        };
        if names.contains(&identity.trim()) {
            nodes.push(dev.join(&event));
        }
    }
    nodes.sort();
    nodes
}

pub fn open_named(names: &[&str]) -> Vec<File> {
    named_nodes(
        Path::new("/sys/class/input"),
        Path::new("/dev/input"),
        names,
    )
    .into_iter()
    .filter_map(|path| {
        match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
        {
            Ok(file) => {
                println!("couch-gui: input {}", path.display());
                Some(file)
            }
            Err(error) => {
                eprintln!("couch-gui: input {}: {error}", path.display());
                None
            }
        }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reordered_devices_are_selected_by_name_without_opening_unrelated_inputs() {
        let root = std::env::temp_dir().join(format!("couch-evdev-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        for (node, name) in [
            ("event1", "ACCDET"),
            ("event7", "mt_gpio_kpd"),
            ("event2", "mtk-tpd"),
            ("event9", "mtk-kpd"),
        ] {
            let dir = root.join(node).join("device");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("name"), format!("{name}\n")).unwrap();
        }
        let nodes = named_nodes(&root, Path::new("/dev/input"), &["mt_gpio_kpd", "mtk-kpd"]);
        fs::remove_dir_all(root).unwrap();
        assert_eq!(
            nodes,
            vec![
                PathBuf::from("/dev/input/event7"),
                PathBuf::from("/dev/input/event9")
            ]
        );
    }
}
