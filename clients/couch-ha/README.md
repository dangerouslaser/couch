# Home Assistant client

Blocking Rust REST client for lights, covers (including blinds), and climate
entities. Run it on a worker. Credentials remain in private connection settings;
errors and `Debug` never print tokens or server response bodies.

`entities()` discovers all supported domains from one `/api/states` snapshot.
`lights()`, `covers()`, `climates()` and individual entity getters are also
available. Existing light commands retain their API.

Cover commands use `open_cover`, `close_cover`, `stop_cover`, and
`set_cover_position`. Position is **0 closed, 100 open**. Toggle reverses direction
when moving; open/opening closes, closed/closing opens. Advertised feature flags
and current availability are checked before posting. Tilt is not exposed yet.

Climate state includes current and target temperatures, heating/cooling range,
HVAC mode/action, supported modes, limits, and target step. HA's `/api/config`
temperature unit is cached for five minutes per client: state attributes and
service values use this configured unit, not necessarily the hardware's native
unit. Unknown units fail instead of guessing. A missing step defaults to 0.5°C
or 1°F; missing bounds disable temperature changes.

`Climate::adjusted_target` accepts a signed step count and optional pending
command for repeated keys. It clamps to advertised limits; in `heat_cool` it
shifts both bounds while preserving their gap. It refuses unknown targets and
inactive, automatic, dry, or fan-only modes. Mode changes are explicit commands
and must occur in the entity's advertised mode list. Command success acknowledges
the HA service, not immediate physical completion; refresh state afterward.

Validation: `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0
CARGO_PROFILE_TEST_DEBUG=0 cargo test -p couch-ha` from `clients/`. Mock HTTP tests
verify service payloads, unavailable/unsupported rejection, units, discovery,
authentication, and target adjustment. Physical blind/thermostat testing remains
separate.

Contracts: [cover entity](https://developers.home-assistant.io/docs/core/entity/cover/),
[climate entity](https://developers.home-assistant.io/docs/core/entity/climate/),
[climate actions](https://www.home-assistant.io/integrations/climate/), and
[HA climate REST conversion](https://github.com/home-assistant/core/blob/dev/homeassistant/components/climate/__init__.py).
