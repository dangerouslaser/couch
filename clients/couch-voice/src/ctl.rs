//! The mixer, over `/dev/snd/controlC<card>`.
//!
//! This exists because on this board opening the capture device is not enough.
//! MediaTek's mt6580 ASoC driver barely uses DAPM: the analogue path is
//! switched by kcontrol `put` handlers that call the PMIC codec directly, so a
//! PCM device can be open, clocked and delivering frames while the ADC in
//! front of it is powered down. The frames are then real, correctly timed, and
//! silent - which is the single most misleading failure available on this
//! hardware, because it looks exactly like a microphone that is not connected.
//!
//! Android's own audio HAL does the same thing through the same names:
//! `Audio_ADC_1_Switch`, `Audio_Preamp1_Switch`, `Audio_MicSource1_Setting`,
//! `Audio_PGA1_Setting`, `Audio_MIC1_Mode_Select`. Those strings are in both
//! the shipped kernel and `vendor/lib/libaudio.primary.so`, which is where
//! this list came from - see `docs/voice.md`.
//!
//! Nothing here guesses a route. `Control` reads and writes named elements and
//! the probe prints what it finds, because a route invented on a laptop and
//! applied to somebody's living room is a worse answer than a list of what the
//! card actually has.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;

use crate::abi::{self, ElemId, ElemInfo, ElemList, ElemValue};
use crate::error::{Error, Result};

/// What the card calls itself.
#[derive(Debug, Clone)]
pub struct Card {
    pub index: i32,
    pub id: String,
    pub driver: String,
    pub name: String,
    pub longname: String,
    pub mixername: String,
    pub components: String,
}

/// One value of one control.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    /// An enumerated control's current item, by index and by the name the
    /// driver gives it.
    Enum(u32, String),
    Bytes(usize),
    /// A type this crate does not decode - IEC958, or a 64-bit integer.
    Opaque,
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{}", if *b { "on" } else { "off" }),
            Value::Int(n) => write!(f, "{n}"),
            Value::Enum(i, name) => write!(f, "{name} ({i})"),
            Value::Bytes(n) => write!(f, "<{n} bytes>"),
            Value::Opaque => write!(f, "<opaque>"),
        }
    }
}

/// One mixer control, with everything a human needs to decide what to set it
/// to: its current value, and for an enumerated control every item it accepts.
#[derive(Debug, Clone)]
pub struct Element {
    pub numid: u32,
    pub name: String,
    pub index: u32,
    pub count: u32,
    pub kind: &'static str,
    pub writable: bool,
    pub inactive: bool,
    pub values: Vec<Value>,
    /// Every item of an enumerated control, in order.
    pub items: Vec<String>,
    /// min, max, step of an integer control.
    pub range: Option<(i64, i64, i64)>,
}

impl Element {
    /// One line, wide enough to read and narrow enough for a serial console.
    pub fn line(&self) -> String {
        let values = self
            .values
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let mut s = format!(
            "{:>4}  {:<34} {:<11} {}",
            self.numid, self.name, self.kind, values
        );
        if let Some((min, max, step)) = self.range {
            s.push_str(&format!("   [{min}..{max} step {step}]"));
        }
        if !self.items.is_empty() {
            s.push_str(&format!("   {{{}}}", self.items.join(" | ")));
        }
        if !self.writable {
            s.push_str("   (read-only)");
        }
        if self.inactive {
            s.push_str("   (inactive)");
        }
        s
    }
}

pub struct Control {
    file: File,
    path: String,
}

impl Control {
    pub fn open(card: u32) -> Result<Control> {
        let path = format!("/dev/snd/controlC{card}");
        // Read-write, because setting a route is the point. A card that is
        // only readable is worth failing on here rather than three calls later.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| Error::Alsa {
                device: path.clone(),
                call: "open",
                source: e,
            })?;
        Ok(Control { file, path })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn protocol(&self) -> Result<(u32, u32, u32)> {
        let mut v: i32 = 0;
        self.call(
            "PVERSION",
            abi::CTL_IOCTL_PVERSION,
            &mut v as *mut _ as *mut _,
        )?;
        let v = v as u32;
        Ok(((v >> 16) & 0xffff, (v >> 8) & 0xff, v & 0xff))
    }

    pub fn card(&self) -> Result<Card> {
        let mut info: abi::CardInfo = unsafe { std::mem::zeroed() };
        self.call(
            "CARD_INFO",
            abi::CTL_IOCTL_CARD_INFO,
            &mut info as *mut _ as *mut _,
        )?;
        Ok(Card {
            index: info.card,
            id: abi::cstr(&info.id),
            driver: abi::cstr(&info.driver),
            name: abi::cstr(&info.name),
            longname: abi::cstr(&info.longname),
            mixername: abi::cstr(&info.mixername),
            components: abi::cstr(&info.components),
        })
    }

    /// Every control on the card, with its current value.
    pub fn elements(&self) -> Result<Vec<Element>> {
        self.ids()?
            .into_iter()
            .map(|id| self.describe(id))
            .collect()
    }

    /// One control by name, e.g. `Audio_ADC_1_Switch`.
    pub fn get(&self, name: &str, index: u32) -> Result<Element> {
        let mut id = ElemId {
            index,
            ..ElemId::default()
        };
        abi::set_name(&mut id.name, name);
        self.describe(id)
    }

    /// Set every value of a control from one string. Booleans take
    /// on/off/1/0/true/false, integers a number, and enumerated controls
    /// either an item name (case-insensitively) or its index.
    ///
    /// All `count` values are set to the same thing. Every control on this
    /// card's capture path has a count of one, and a partial write - channel
    /// one routed and channel two not - is a fault that is very hard to see.
    pub fn set(&self, name: &str, index: u32, wanted: &str) -> Result<Element> {
        let current = self.get(name, index)?;
        if !current.writable {
            return Err(Error::Audio {
                device: self.path.clone(),
                detail: format!("{name} is read-only"),
            });
        }
        let mut value = ElemValue {
            id: ElemId {
                index,
                ..ElemId::default()
            },
            ..ElemValue::default()
        };
        abi::set_name(&mut value.id.name, name);

        let count = current.count.max(1) as usize;
        match current.kind {
            "BOOLEAN" => {
                let on = match wanted.to_ascii_lowercase().as_str() {
                    "on" | "1" | "true" | "yes" => true,
                    "off" | "0" | "false" | "no" => false,
                    _ => {
                        return Err(Error::Audio {
                            device: self.path.clone(),
                            detail: format!("{name} is a switch; {wanted:?} is not on or off"),
                        })
                    }
                };
                for i in 0..count {
                    value.set_long_at(i, on as i64);
                }
            }
            "INTEGER" => {
                let n: i64 = wanted.parse().map_err(|_| Error::Audio {
                    device: self.path.clone(),
                    detail: format!("{name} takes a number; {wanted:?} is not one"),
                })?;
                if let Some((min, max, _)) = current.range {
                    if n < min || n > max {
                        return Err(Error::Audio {
                            device: self.path.clone(),
                            detail: format!("{name} takes {min}..{max}, not {n}"),
                        });
                    }
                }
                for i in 0..count {
                    value.set_long_at(i, n);
                }
            }
            "ENUMERATED" => {
                let item = current
                    .items
                    .iter()
                    .position(|i| i.eq_ignore_ascii_case(wanted))
                    .map(|i| i as u32)
                    .or_else(|| {
                        wanted
                            .parse::<u32>()
                            .ok()
                            .filter(|n| (*n as usize) < current.items.len())
                    })
                    .ok_or_else(|| Error::Audio {
                        device: self.path.clone(),
                        detail: format!(
                            "{name} takes one of {}; not {wanted:?}",
                            current.items.join(", ")
                        ),
                    })?;
                for i in 0..count {
                    value.set_u32_at(i, item);
                }
            }
            other => {
                return Err(Error::Audio {
                    device: self.path.clone(),
                    detail: format!("{name} is a {other} control, which this cannot set"),
                })
            }
        }

        self.call(
            "ELEM_WRITE",
            abi::CTL_IOCTL_ELEM_WRITE,
            &mut value as *mut _ as *mut _,
        )?;
        // Read it back rather than reporting what we asked for. A driver is
        // free to clamp, and on this codec several controls do.
        self.get(name, index)
    }

    // --- internals ----------------------------------------------------------

    fn ids(&self) -> Result<Vec<ElemId>> {
        let mut list = ElemList {
            offset: 0,
            space: 0,
            used: 0,
            count: 0,
            pids: std::ptr::null_mut(),
            reserved: [0; 50],
        };
        self.call(
            "ELEM_LIST",
            abi::CTL_IOCTL_ELEM_LIST,
            &mut list as *mut _ as *mut _,
        )?;

        let count = list.count as usize;
        let mut ids = vec![ElemId::default(); count];
        if count == 0 {
            return Ok(ids);
        }
        list.offset = 0;
        list.space = count as u32;
        list.pids = ids.as_mut_ptr();
        self.call(
            "ELEM_LIST",
            abi::CTL_IOCTL_ELEM_LIST,
            &mut list as *mut _ as *mut _,
        )?;
        ids.truncate(list.used as usize);
        Ok(ids)
    }

    fn describe(&self, id: ElemId) -> Result<Element> {
        let mut info = ElemInfo {
            id,
            ..ElemInfo::default()
        };
        self.call(
            "ELEM_INFO",
            abi::CTL_IOCTL_ELEM_INFO,
            &mut info as *mut _ as *mut _,
        )?;

        let kind = match info.ty {
            abi::ELEM_TYPE_BOOLEAN => "BOOLEAN",
            abi::ELEM_TYPE_INTEGER => "INTEGER",
            abi::ELEM_TYPE_ENUMERATED => "ENUMERATED",
            abi::ELEM_TYPE_BYTES => "BYTES",
            abi::ELEM_TYPE_INTEGER64 => "INTEGER64",
            _ => "OTHER",
        };
        let count = info.count;
        let range = (info.ty == abi::ELEM_TYPE_INTEGER).then(|| info.integer_range());

        // Item names come back one at a time: write the item number into the
        // union, ask again, read the name out. The struct is rebuilt each time
        // because ELEM_INFO overwrites all of it.
        let mut items = Vec::new();
        if info.ty == abi::ELEM_TYPE_ENUMERATED {
            for item in 0..info.enum_items() {
                let mut probe = ElemInfo {
                    id: info.id,
                    ..ElemInfo::default()
                };
                probe.set_enum_item(item);
                if self
                    .call(
                        "ELEM_INFO",
                        abi::CTL_IOCTL_ELEM_INFO,
                        &mut probe as *mut _ as *mut _,
                    )
                    .is_err()
                {
                    break;
                }
                items.push(probe.enum_name());
            }
        }

        let readable = info.access & abi::ELEM_ACCESS_READ != 0;
        let mut values = Vec::new();
        if readable {
            let mut value = ElemValue {
                id: info.id,
                ..ElemValue::default()
            };
            if self
                .call(
                    "ELEM_READ",
                    abi::CTL_IOCTL_ELEM_READ,
                    &mut value as *mut _ as *mut _,
                )
                .is_ok()
            {
                for i in 0..count as usize {
                    values.push(match info.ty {
                        abi::ELEM_TYPE_BOOLEAN => Value::Bool(value.long_at(i) != 0),
                        abi::ELEM_TYPE_INTEGER => Value::Int(value.long_at(i)),
                        abi::ELEM_TYPE_ENUMERATED => {
                            let n = value.u32_at(i);
                            Value::Enum(
                                n,
                                items.get(n as usize).cloned().unwrap_or_else(|| "?".into()),
                            )
                        }
                        abi::ELEM_TYPE_BYTES => Value::Bytes(count as usize),
                        _ => Value::Opaque,
                    });
                    // A byte array is one value, not `count` of them.
                    if info.ty == abi::ELEM_TYPE_BYTES {
                        break;
                    }
                }
            }
        }

        Ok(Element {
            numid: info.id.numid,
            name: abi::cstr(&info.id.name),
            index: info.id.index,
            count,
            kind,
            writable: info.access & abi::ELEM_ACCESS_WRITE != 0,
            inactive: info.access & abi::ELEM_ACCESS_INACTIVE != 0,
            values,
            items,
            range,
        })
    }

    fn call(&self, name: &'static str, request: u32, arg: *mut libc::c_void) -> Result<i32> {
        unsafe { abi::ioctl(self.file.as_raw_fd(), request, arg) }.map_err(|e| Error::Alsa {
            device: self.path.clone(),
            call: name,
            source: e,
        })
    }
}

/// The controls Android's audio HAL touches to bring up this board's single
/// analogue microphone, in the order it touches them, with the values it uses
/// where those are known and a marker where they are not.
///
/// This is a starting point for `couch-mic route`, not a fact: the names come
/// from the shipped kernel and the shipped HAL, but which enumerated item is
/// the right one is a question only the hardware can answer. `couch-mic
/// controls` prints the items; `couch-mic set` tries them.
pub const MTK_AMIC_ROUTE: &[(&str, &str)] = &[
    // The clock buffer for the analogue side. Nothing downstream works
    // without it.
    ("AUD_CLK_BUF_Switch", "on"),
    // Which physical input the preamp listens to. `MTK_DIGITAL_MIC_SUPPORT`
    // is "no" on this build and `MTK_AUDIO_NUMBER_OF_MIC` is 1, so this is an
    // analogue capsule on the first input.
    ("Audio_MicSource1_Setting", "ADC1"),
    // Analogue capacitive coupling. The vendor config carries an
    // `Audio_AMIC_DCC_Setting`, so DCCMODE is the other candidate.
    ("Audio_MIC1_Mode_Select", "ACCMODE"),
    // NOT "on": this control's items are OPEN / IN_ADC1 / IN_ADC2 / IN_ADC3,
    // and it is the switch that actually connects the capsule to the ADC. Set
    // to "on" it is rejected, and because every other line in this table
    // succeeds, routing then reports success with the microphone still
    // disconnected. Measured on the HA100: IN_ADC1 is the only input that
    // behaves like a capsule - IN_ADC2 and IN_ADC3 sit at -0.3 dBFS, clipping,
    // which is a floating pin pulled to a rail.
    ("Audio_Preamp1_Switch", "IN_ADC1"),
    ("Audio_ADC_1_Switch", "on"),
    // Maximum, not zero. This codec has no digital capture gain - all 97 of
    // its controls were checked - so this preamp is the only gain in the
    // path, and a remote across a room needs every dB of it. Measured at 24dB:
    // noise floor -36 dBFS mean peak, a close tap 14 dB above it.
    ("Audio_PGA1_Setting", "24Db"),
];
