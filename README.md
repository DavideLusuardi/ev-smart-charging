# ev-smart-charging

A Charging Station Management System (CSMS) in Rust that speaks OCPP 2.0.1 and shares a
site's limited grid power across connected EVs with a scheduling optimizer.

> **Status:** work in progress.

## Crates

| Crate | Purpose |
|---|---|
| `crates/ocpp` | OCPP-J RPC framing and the OCPP 2.0.1 message subset used for smart charging |
| `crates/optimizer` | Pure scheduling library (equal share, EDF, LP) and the receding-horizon planner |
| `crates/csms` | Server binary: WebSocket endpoint, control loop, REST API, persistence, metrics |
| `crates/simulator` | Binary `sim`: live simulated stations and offline benchmark runs |

## Building

```sh
cargo build --workspace
cargo test --workspace
```

## Running the CSMS

```sh
cargo run -p csms -- --listen 0.0.0.0:9000
```

Stations connect to `ws://localhost:9000/ocpp/{stationId}` with the `ocpp2.0.1`
subprotocol. `cargo run -p csms -- --help` lists the options (heartbeat interval, call
timeout, id token allowlist); `RUST_LOG=csms=debug` logs every frame. The supported messages
and connection rules are in [docs/ocpp-subset.md](docs/ocpp-subset.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
