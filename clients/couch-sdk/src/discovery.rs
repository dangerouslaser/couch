//! Finding devices, optionally.
//!
//! Discovery in this repository is deliberately not a client concern today: the
//! daemon owns the mDNS browser (`mdns_sd` in `api/streaming_tv.rs`) so that one
//! process, not five, holds a multicast socket on a device with this much RAM.
//! What a client owns is the *service name* it should be browsed for and the
//! translation from a found address into its own settings.
//!
//! Implement [`Discover`] only if your protocol advertises itself. A client
//! without it is not second-class - Denon and Kodi are both entered by hand -
//! and nothing in the UI requires it.

use serde::{Deserialize, Serialize};

use crate::{DeviceClient, Result};

/// One device found on the network, in the shape the daemon's discovery
/// endpoints already return to the browser.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discovered {
    /// What the device calls itself. Display only; never used as an ID.
    pub name: String,
    pub address: String,
    pub port: u16,
}

impl Discovered {
    pub fn new(name: impl Into<String>, address: impl Into<String>, port: u16) -> Self {
        Self {
            name: name.into(),
            address: address.into(),
            port,
        }
    }
}

pub trait Discover: DeviceClient {
    /// The DNS-SD service this provider advertises, e.g.
    /// `"_androidtvremote2._tcp.local."`. The daemon browses it; the client
    /// does not open a multicast socket of its own.
    const MDNS_SERVICE: &'static str;

    /// Turn a found device into settings, or explain why it cannot be used
    /// without more input - a device needing a pairing code is [`crate::Error::Invalid`]
    /// here, and the UI asks for the code.
    fn settings_for(found: &Discovered) -> Result<Self::Settings>;
}
