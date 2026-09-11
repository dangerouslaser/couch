//! A whole session against a television that does not exist.
//!
//! `cargo run -p couch-echo --example demo`
//!
//! No hardware, no credentials, no network beyond loopback. The point is that
//! the success path and both failure paths are visible in one run: a command
//! the device accepts, one it refuses with a reason, one this client refuses
//! before it touches the socket, and one the device never answers at all.

use couch_sdk::testing::{MockHost, Reply, Script};
use couch_sdk::{ClientSettings, DeviceClient};

use couch_echo::{EchoTv, Settings};

fn main() {
    let host = MockHost::start(
        Script::new()
            .terminator(b'\n')
            .on("CMD volume-up", Reply::line("OK"))
            .on(
                "GET STATUS",
                Reply::line("STATUS power=on;mute=off;volume=31;input=hdmi1;loudness=high"),
            )
            .on(
                "LIST INPUTS",
                Reply::Lines(vec![
                    "INPUT hdmi1 Blu-ray".into(),
                    "INPUT hdmi2 Console".into(),
                    "END".into(),
                ]),
            )
            .on("CMD input:hdmi2", Reply::line("OK"))
            .on("CMD power-off", Reply::line("ERR the TV is locked"))
            // Everything else is met with silence, which is how a real device
            // behaves when its network stack is asleep.
            .otherwise(Reply::Silence),
    );

    let settings = Settings {
        host: host.host().into(),
        port: host.port(),
        token: "example-pairing-token".into(),
    };
    settings.validate().expect("settings are addressable");

    let mut tv = EchoTv::connect(&settings).expect("the fake TV is listening");

    println!("volume-up          -> {:?}", tv.command("volume-up"));
    println!("status             -> {:?}", tv.status());
    println!("inputs             -> {:?}", tv.inputs());
    println!("input:hdmi2        -> {:?}", tv.command("input:hdmi2"));

    // The device understood and said no; its own words reach the user.
    println!("power-off          -> {:?}", tv.command("power-off"));

    // Never declared, so it is refused here and costs no round trip.
    println!("fast-forward       -> {:?}", tv.command("fast-forward"));
    println!("not-a-function     -> {:?}", tv.command("wash-the-dishes"));

    // An input ID this client would not accept from a device either.
    println!("input:../escape    -> {:?}", tv.command("input:../escape"));

    // Unscripted: the TV says nothing and the deadline expires. Not retried,
    // because a lost reply does not prove a lost command.
    println!("home (no reply)    -> {:?}", tv.command("home"));

    println!("\nthe TV was asked: {:?}", host.requests());
}
