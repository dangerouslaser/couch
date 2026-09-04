//! Stable identifiers, derived from names but not tied to them.
//!
//! Everything a user can rename is referenced by id, so renaming "Study" to
//! "Office" does not silently detach it from three areas. The ids are slugs
//! rather than numbers or UUIDs because the config file is meant to be
//! readable and hand-editable: `"rooms": ["living-room", "kitchen"]` says what
//! it is, `[3, 7]` does not.

use alloc::borrow::ToOwned;
use alloc::string::String;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(String);

impl Id {
    pub fn new(s: impl Into<String>) -> Id {
        Id(s.into())
    }

    /// A slug: lowercase, alphanumerics kept, everything else a single dash.
    ///
    /// Names with no slug-able characters at all - an emoji-only room name is
    /// the realistic case - would produce an empty id, which would then collide
    /// with the next one, so they fall back to a fixed stem and let
    /// [`Id::unique`] number them.
    pub fn from_name(name: &str) -> Id {
        let mut s = String::with_capacity(name.len());
        let mut pending_dash = false;
        for ch in name.chars() {
            if ch.is_ascii_alphanumeric() {
                if pending_dash && !s.is_empty() {
                    s.push('-');
                }
                pending_dash = false;
                s.push(ch.to_ascii_lowercase());
            } else {
                pending_dash = true;
            }
        }
        if s.is_empty() {
            s.push_str("item");
        }
        Id(s)
    }

    /// [`Id::from_name`], then `-2`, `-3`, ... until nothing else has it.
    pub fn unique<'a>(name: &str, taken: impl Iterator<Item = &'a Id> + Clone) -> Id {
        let base = Id::from_name(name);
        if !taken.clone().any(|id| *id == base) {
            return base;
        }
        let mut n = 2u32;
        loop {
            let mut candidate = base.0.clone();
            candidate.push('-');
            crate::push_u64(&mut candidate, n as u64);
            let candidate = Id(candidate);
            if !taken.clone().any(|id| *id == candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&str> for Id {
    fn from(s: &str) -> Id {
        Id(s.to_owned())
    }
}

impl From<String> for Id {
    fn from(s: String) -> Id {
        Id(s)
    }
}

impl AsRef<str> for Id {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for Id {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn slugs_collapse_punctuation() {
        assert_eq!(Id::from_name("Living room").as_str(), "living-room");
        assert_eq!(Id::from_name("  Kids' Room!  ").as_str(), "kids-room");
        assert_eq!(Id::from_name("LG C3").as_str(), "lg-c3");
        assert_eq!(Id::from_name("---").as_str(), "item");
    }

    #[test]
    fn unique_numbers_collisions() {
        let taken = vec![Id::new("kitchen"), Id::new("kitchen-2")];
        assert_eq!(Id::unique("Kitchen", taken.iter()).as_str(), "kitchen-3");
    }
}
