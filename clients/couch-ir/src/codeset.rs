//! Mapping a button to a protocol, address and command.
//!
//! `couch-model`'s `Integration::Ir { codeset }` names *which* remote's codes a
//! device answers to, but not the codes themselves - deliberately, so the model
//! crate (which also builds for wasm) never carries a table of IR values. This
//! is where the name resolves to codes: a small line-oriented file the daemon
//! or the CLI loads.
//!
//! ```text
//! # lg-tv.codeset  - one button per line:
//! #   <button> <protocol> <address> <command>
//! power   nec   0x04 0x08
//! volup   nec   0x04 0x02
//! voldn   nec   0x04 0x03
//! mute    nec   0x04 0x09
//! input   nec   0x04 0x0b
//! ```
//!
//! The button names are free text on purpose, but the intended vocabulary is
//! `couch-model`'s: an `Action.command` of `on`/`off`/`play`/`volume:up` maps
//! to a button here. This crate does **not** depend on `couch-model` - the same
//! separation `couch-voice` keeps, and the reason the model stays wasm-clean -
//! so the correspondence is a convention, documented in `docs/ir.md`, not a
//! shared type. Resolving `codeset` to a file is likewise the daemon's job; the
//! documented convention is `/opt/couch/ir/<codeset>.codeset`.
//!
//! Nothing in `couch-model` changes for any of this. `Integration::Ir` already
//! carries exactly the one field - the codeset name - that this needs.

use crate::error::{Error, Result};
use crate::proto::{self, Message, Protocol};

/// One button's codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub button: String,
    pub protocol: Protocol,
    pub address: u32,
    pub command: u32,
}

impl Entry {
    /// Encode this button into a sendable message. `toggle` is only consulted
    /// by RC5/RC6.
    pub fn encode(&self, toggle: bool) -> Result<Message> {
        proto::encode(self.protocol, self.address, self.command, toggle)
    }
}

/// A parsed codeset: buttons in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Codeset {
    pub entries: Vec<Entry>,
}

impl Codeset {
    /// Parse a codeset. `path` is only for error messages.
    pub fn parse(path: &str, text: &str) -> Result<Codeset> {
        let mut entries = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            // Strip comments (everything after `#`) and surrounding space.
            let content = raw.split('#').next().unwrap_or("").trim();
            if content.is_empty() {
                continue;
            }
            let fields: Vec<&str> = content.split_whitespace().collect();
            if fields.len() != 4 {
                return Err(Error::Codeset {
                    path: path.to_string(),
                    line,
                    detail: format!(
                        "expected `<button> <protocol> <address> <command>`, got {} field(s)",
                        fields.len()
                    ),
                });
            }
            let protocol = Protocol::from_name(fields[1]).ok_or_else(|| Error::Codeset {
                path: path.to_string(),
                line,
                detail: format!("unknown protocol {:?}", fields[1]),
            })?;
            if protocol == Protocol::Raw {
                return Err(Error::Codeset {
                    path: path.to_string(),
                    line,
                    detail: "raw has no address/command; it cannot appear in a codeset".into(),
                });
            }
            let address = parse_u32(fields[2]).map_err(|e| Error::Codeset {
                path: path.to_string(),
                line,
                detail: format!("address: {e}"),
            })?;
            let command = parse_u32(fields[3]).map_err(|e| Error::Codeset {
                path: path.to_string(),
                line,
                detail: format!("command: {e}"),
            })?;
            // Reject codes the protocol cannot hold, at load time rather than
            // at the first press months later.
            proto::encode(protocol, address, command, false).map_err(|e| Error::Codeset {
                path: path.to_string(),
                line,
                detail: e.to_string(),
            })?;
            entries.push(Entry {
                button: fields[0].to_string(),
                protocol,
                address,
                command,
            });
        }
        Ok(Codeset { entries })
    }

    /// Find a button by name (case-insensitive, since a codeset author and a
    /// caller need not agree on capitalisation).
    pub fn get(&self, button: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.button.eq_ignore_ascii_case(button))
    }

    /// The button names, in file order, for a "no such button" message.
    pub fn buttons(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.button.as_str()).collect()
    }
}

/// Parse a `u32` written as decimal, `0x`-hex or `0b`-binary. Shared by the CLI
/// for addresses and commands so `send`, `raw` and codesets all read numbers
/// the same way.
pub fn parse_u32(s: &str) -> std::result::Result<u32, String> {
    let t = s.trim();
    let parsed = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(h, 16)
    } else if let Some(b) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        u32::from_str_radix(b, 2)
    } else {
        t.parse::<u32>()
    };
    parsed.map_err(|_| format!("{s:?} is not a number"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# a sample TV
power   nec   0x04 0x08
volup   nec   0x04 0x02   # inline comment
MUTE    rc5   0x00 0x0d

sony    sony12 1 21
";

    #[test]
    fn parses_and_looks_up() {
        let cs = Codeset::parse("tv.codeset", SAMPLE).unwrap();
        assert_eq!(cs.entries.len(), 4);
        let p = cs.get("power").unwrap();
        assert_eq!(p.protocol, Protocol::Nec);
        assert_eq!((p.address, p.command), (0x04, 0x08));
        // Case-insensitive lookup, and inline comments stripped.
        assert_eq!(cs.get("mute").unwrap().protocol, Protocol::Rc5);
        assert_eq!(cs.get("volup").unwrap().command, 0x02);
        assert!(cs.get("nope").is_none());
    }

    #[test]
    fn an_entry_encodes() {
        let cs = Codeset::parse("tv.codeset", SAMPLE).unwrap();
        let m = cs.get("power").unwrap().encode(false).unwrap();
        assert_eq!(m.frame.carrier_hz, 38_000);
    }

    #[test]
    fn rejects_bad_lines() {
        assert!(Codeset::parse("x", "power nec 0x04").is_err()); // too few fields
        assert!(Codeset::parse("x", "power bogus 1 2").is_err()); // unknown protocol
        assert!(Codeset::parse("x", "power nec 0x100 2").is_err()); // out of range
        assert!(Codeset::parse("x", "power raw 1 2").is_err()); // raw not allowed
    }

    #[test]
    fn number_formats() {
        assert_eq!(parse_u32("16"), Ok(16));
        assert_eq!(parse_u32("0x10"), Ok(16));
        assert_eq!(parse_u32("0b10000"), Ok(16));
        assert!(parse_u32(" zz").is_err());
    }
}
