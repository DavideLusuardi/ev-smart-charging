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

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
