//! Field numbers follow Android Remote v2 and Google's Polo pairing schema.
//! Unknown protobuf fields are ignored for forward compatibility.
use prost::Message;
#[derive(Clone, PartialEq, Message)]
pub struct Pair {
    #[prost(uint32, tag = "1")]
    pub version: u32,
    #[prost(uint32, tag = "2")]
    pub status: u32,
    #[prost(message, optional, tag = "10")]
    pub request: Option<Request>,
    #[prost(message, optional, tag = "11")]
    pub request_ack: Option<Empty>,
    #[prost(message, optional, tag = "20")]
    pub options: Option<Options>,
    #[prost(message, optional, tag = "30")]
    pub configuration: Option<Configuration>,
    #[prost(message, optional, tag = "31")]
    pub configuration_ack: Option<Empty>,
    #[prost(message, optional, tag = "40")]
    pub secret: Option<Secret>,
    #[prost(message, optional, tag = "41")]
    pub secret_ack: Option<Secret>,
}
#[derive(Clone, PartialEq, Message)]
pub struct Empty {}
#[derive(Clone, PartialEq, Message)]
pub struct Request {
    #[prost(string, tag = "1")]
    pub service: String,
    #[prost(string, tag = "2")]
    pub client: String,
}
#[derive(Clone, PartialEq, Message)]
pub struct Encoding {
    #[prost(uint32, tag = "1")]
    pub kind: u32,
    #[prost(uint32, tag = "2")]
    pub length: u32,
}
#[derive(Clone, PartialEq, Message)]
pub struct Options {
    #[prost(message, repeated, tag = "1")]
    pub input: Vec<Encoding>,
    #[prost(message, repeated, tag = "2")]
    pub output: Vec<Encoding>,
    #[prost(uint32, tag = "3")]
    pub role: u32,
}
#[derive(Clone, PartialEq, Message)]
pub struct Configuration {
    #[prost(message, optional, tag = "1")]
    pub encoding: Option<Encoding>,
    #[prost(uint32, tag = "2")]
    pub role: u32,
}
#[derive(Clone, PartialEq, Message)]
pub struct Secret {
    #[prost(bytes = "vec", tag = "1")]
    pub value: Vec<u8>,
}
#[derive(Clone, PartialEq, Message)]
pub struct Remote {
    #[prost(message, optional, tag = "1")]
    pub configure: Option<Configure>,
    #[prost(message, optional, tag = "2")]
    pub active: Option<Number>,
    #[prost(message, optional, tag = "3")]
    pub error: Option<Empty>,
    #[prost(message, optional, tag = "8")]
    pub ping: Option<Number>,
    #[prost(message, optional, tag = "9")]
    pub pong: Option<Number>,
    #[prost(message, optional, tag = "10")]
    pub key: Option<Key>,
    #[prost(message, optional, tag = "40")]
    pub start: Option<Started>,
    #[prost(message, optional, tag = "50")]
    pub volume: Option<Volume>,
    #[prost(message, optional, tag = "90")]
    pub app: Option<AppLink>,
}
#[derive(Clone, PartialEq, Message)]
pub struct Configure {
    #[prost(uint32, tag = "1")]
    pub features: u32,
    #[prost(message, optional, tag = "2")]
    pub device: Option<Device>,
}
#[derive(Clone, PartialEq, Message)]
pub struct Device {
    #[prost(string, tag = "1")]
    pub model: String,
    #[prost(string, tag = "2")]
    pub vendor: String,
    #[prost(uint32, tag = "3")]
    pub unknown1: u32,
    #[prost(string, tag = "4")]
    pub unknown2: String,
    #[prost(string, tag = "5")]
    pub package: String,
    #[prost(string, tag = "6")]
    pub version: String,
}
#[derive(Clone, PartialEq, Message)]
pub struct Number {
    #[prost(uint32, tag = "1")]
    pub value: u32,
}
#[derive(Clone, PartialEq, Message)]
pub struct Key {
    #[prost(uint32, tag = "1")]
    pub code: u32,
    #[prost(uint32, tag = "2")]
    pub direction: u32,
}
#[derive(Clone, PartialEq, Message)]
pub struct Started {
    #[prost(bool, tag = "1")]
    pub on: bool,
}
#[derive(Clone, PartialEq, Message)]
pub struct Volume {
    #[prost(uint32, tag = "6")]
    pub max: u32,
    #[prost(uint32, tag = "7")]
    pub level: u32,
    #[prost(bool, tag = "8")]
    pub muted: bool,
}
#[derive(Clone, PartialEq, Message)]
pub struct AppLink {
    #[prost(string, tag = "1")]
    pub url: String,
}
