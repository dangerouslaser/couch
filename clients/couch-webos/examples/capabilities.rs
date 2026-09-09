//! Read-only capability probe. No playback or settings mutations.
use couch_webos::{Client, Settings};
use serde_json::json;
fn main() {
    let path = std::env::args().nth(1).expect("private settings file");
    let settings = Settings::load(std::path::Path::new(&path)).unwrap();
    let mut c = Client::connect(&settings).unwrap();
    for (name, uri, payload) in [
        ("system", "ssap://system/getSystemInfo", json!({})),
        (
            "software",
            "ssap://com.webos.service.update/getCurrentSWInformation",
            json!({}),
        ),
        (
            "foreground",
            "ssap://com.webos.applicationManager/getForegroundAppInfo",
            json!({}),
        ),
        (
            "media",
            "ssap://com.webos.media/getForegroundAppInfo",
            json!({}),
        ),
        (
            "picture",
            "ssap://settings/getSystemSettings",
            json!({"category":"picture","keys":["pictureMode","backlight","brightness","contrast"]}),
        ),
        (
            "sound",
            "ssap://com.webos.service.apiadapter/audio/getSoundOutput",
            json!({}),
        ),
        ("channel", "ssap://tv/getCurrentChannel", json!({})),
        ("inputs", "ssap://tv/getExternalInputList", json!({})),
    ] {
        match c.request(uri, payload) {
            Ok(v) => println!("{}", json!({"probe":name,"data":v})),
            Err(e) => println!("{}", json!({"probe":name,"error":e.to_string()})),
        }
    }
}
