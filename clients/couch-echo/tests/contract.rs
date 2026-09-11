//! What a contributor can prove about a client with no device on the desk.

use couch_sdk::couch_model::Integration;
use couch_sdk::testing::{assert_contract, contract_findings, MockHost, Reply, Script};
use couch_sdk::{
    catalog_differences, DeviceClient, Discover, Discovered, Error, Selectable, Status,
};

use couch_echo::{EchoTv, Settings};

fn script() -> Script {
    Script::new().terminator(b'\n')
}

fn settings(host: &MockHost) -> Settings {
    Settings {
        host: host.host().into(),
        port: host.port(),
        token: "example-pairing-token".into(),
    }
}

#[test]
fn the_client_keeps_the_contract() {
    let host = MockHost::start(script().otherwise(Reply::line("OK")));
    // The mock host itself observes every request, including prohibited ones.
    assert_contract::<EchoTv>(&settings(&host), &host);
}

#[test]
fn a_command_a_reading_and_a_refusal_travel_the_whole_path() {
    let host = MockHost::start(
        script()
            .on("CMD volume-up", Reply::line("OK"))
            .on(
                "GET STATUS",
                Reply::line("STATUS power=on;mute=off;volume=31;input=hdmi1;loudness=high"),
            )
            .on(
                "LIST INPUTS",
                Reply::Lines(vec!["INPUT hdmi1 Blu-ray".into(), "END".into()]),
            )
            .on("CMD power-off", Reply::line("ERR the TV is locked")),
    );
    let mut tv = EchoTv::connect(&settings(&host)).unwrap();

    assert_eq!(tv.command("volume-up"), Ok(()));
    assert_eq!(
        tv.status().unwrap(),
        Status::on(true)
            .with_muted(false)
            .with_volume(31)
            .unwrap()
            .with_input("hdmi1"),
        "an unknown field from newer firmware must not fail the reading"
    );
    assert_eq!(
        tv.inputs().unwrap(),
        vec![Selectable::new("hdmi1", "Blu-ray")]
    );
    // The device's own explanation reaches the user unchanged.
    assert_eq!(
        tv.command("power-off"),
        Err(Error::Remote("the TV is locked".into()))
    );
    assert_eq!(
        host.requests(),
        vec![
            "CMD volume-up",
            "GET STATUS",
            "LIST INPUTS",
            "CMD power-off"
        ]
    );
}

#[test]
fn undeclared_and_unparseable_commands_never_reach_the_device() {
    let host = MockHost::start(script().otherwise(Reply::line("OK")));
    let mut tv = EchoTv::connect(&settings(&host)).unwrap();
    for refused in ["fast-forward", "yellow", "wash-the-dishes", "app:netflix"] {
        assert_eq!(
            tv.command(refused),
            Err(Error::Unsupported),
            "{refused} is not declared and must be refused before any I/O"
        );
    }
    // An input ID that could not be stored safely is refused on the same path.
    assert_eq!(tv.command("input:../escape"), Err(Error::Unsupported));
    assert!(
        host.requests().is_empty(),
        "a refused command must cost no round trip: {:?}",
        host.requests()
    );
}

#[test]
fn silence_becomes_a_timeout_and_a_hang_up_becomes_a_transport_error() {
    let host = MockHost::start(script().on("CMD home", Reply::Silence));
    let mut tv = EchoTv::connect(&settings(&host)).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(tv.command("home"), Err(Error::Timeout));
    assert!(
        started.elapsed() < couch_echo::TIMEOUT * 2,
        "the deadline must bound the wait"
    );
    assert!(
        !Error::Timeout.retryable(),
        "a lost reply is not a lost command"
    );

    let host = MockHost::start(script().on("CMD ok", Reply::Close));
    let mut tv = EchoTv::connect(&settings(&host)).unwrap();
    assert_eq!(tv.command("ok"), Err(Error::Transport));
}

#[test]
fn a_malformed_reading_is_a_protocol_error_rather_than_an_invented_number() {
    for bad in [
        "STATUS volume=300",
        "STATUS power=maybe",
        "STATUS volume",
        "WHAT",
    ] {
        let host = MockHost::start(script().on("GET STATUS", Reply::line(bad)));
        let mut tv = EchoTv::connect(&settings(&host)).unwrap();
        assert_eq!(tv.status(), Err(Error::Protocol), "reading {bad:?}");
    }
}

#[test]
fn a_client_whose_provider_is_not_registered_is_told_so() {
    // couch-echo is deliberately not registered in couch-model, so the catalog
    // the button picker reads offers none of its functions. This is exactly
    // what a contributor sees before they make the seven wiring edits in
    // docs/client-sdk.md, and the check names every one of them.
    let differences = catalog_differences::<EchoTv>(&Integration::None);
    assert_eq!(differences.len(), EchoTv::capabilities().len());
    assert!(differences[0].contains("couch_model::buttons::functions does not offer it"));
}

#[test]
fn an_unreachable_host_is_reported_rather_than_retried() {
    let settings = Settings {
        host: "127.0.0.1".into(),
        // Port 1 on loopback is closed for an unprivileged listener.
        port: 1,
        token: String::new(),
    };
    assert_eq!(
        EchoTv::connect(&settings).err(),
        Some(Error::Transport),
        "a refused connection is a transport failure the user can act on"
    );
    // The checker observes this separate host; the connection failure is the
    // result under test, so no request reaches either host.
    let observer = MockHost::start(script());
    let findings = contract_findings::<EchoTv>(&settings, &observer);
    assert!(findings.iter().any(|f| f.contains("connect failed")));
}

#[test]
fn a_discovered_device_becomes_settings_only_when_it_could_be_reached() {
    assert_eq!(EchoTv::MDNS_SERVICE, "_echotv._tcp.local.");
    let found = Discovered::new("Living room", "10.0.0.42", 9299);
    let settings = EchoTv::settings_for(&found).unwrap();
    assert_eq!((settings.host.as_str(), settings.port), ("10.0.0.42", 9299));
    assert!(
        settings.token.is_empty(),
        "a pairing token is not discoverable and must not be invented"
    );
    // Discovery is untrusted input, so the same validation applies to it.
    for bad in [
        Discovered::new("Broken", "10.0.0.42", 0),
        Discovered::new("Broken", "", 9299),
        Discovered::new("Broken", "10.0.0.42 evil", 9299),
    ] {
        assert_eq!(EchoTv::settings_for(&bad), Err(Error::Invalid));
    }
}
