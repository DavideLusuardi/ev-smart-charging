# Supported OCPP 2.0.1 subset

This project implements the part of OCPP 2.0.1 needed for smart charging: stations report
sessions and EV charging needs, and the CSMS answers with charging profiles. The protocol
types live in [`crates/ocpp`](../crates/ocpp) and are validated against the official OCA
JSON schemas in [`specs/ocpp-2.0.1`](../specs/ocpp-2.0.1) (see `crates/ocpp/tests/schema.rs`).

## Transport and framing

OCPP-J over WebSocket, subprotocol `ocpp2.0.1`. Every WebSocket text message is one frame:

| Frame | Shape |
|---|---|
| CALL | `[2, messageId, action, payload]` |
| CALLRESULT | `[3, messageId, payload]` |
| CALLERROR | `[4, messageId, errorCode, errorDescription, errorDetails]` |

- `messageId` is a string of 1 to 36 characters. Outgoing ids are UUID v4.
- `payload` and `errorDetails` must be JSON objects.
- A CALLRESULT doesn't name its action, so payloads are decoded into typed messages only once
  the response has been matched to its pending CALL.

## Messages

| Action | Direction | Purpose |
|---|---|---|
| `BootNotification` | station → CSMS | Register the station; the reply sets the heartbeat interval |
| `Heartbeat` | station → CSMS | Keep-alive; the reply carries `currentTime` |
| `StatusNotification` | station → CSMS | Connector status per EVSE |
| `Authorize` | station → CSMS | Check an id token before charging |
| `TransactionEvent` | station → CSMS | Session lifecycle (`Started` / `Updated` / `Ended`) and meter values |
| `NotifyEVChargingNeeds` | station → CSMS | ISO 15118 EVs: departure time, energy, max power or current |
| `SetChargingProfile` | CSMS → station | Install a power limit schedule on an EVSE |
| `ClearChargingProfile` | CSMS → station | Remove charging profiles |

Every property of these 16 request and response schemas is modelled, including `customData`,
so any schema-valid message decodes and re-encodes unchanged.

Receiving any other action gives a CALLERROR: `NotSupported` if it belongs to the subset but
is sent by the other side (for example a station receiving `BootNotification`), and
`NotImplemented` otherwise.

## Charging profile conventions

The CSMS sends two kinds of profile:

- **`TxProfile`** for each running transaction: `chargingProfileKind = Absolute`, the
  `transactionId` set, and one `chargingSchedule` with `chargingRateUnit = W`,
  `startSchedule` = the planning time, and one `chargingSchedulePeriod` per change in the
  planned limit.
- **`TxDefaultProfile`** sent after boot: a small fallback limit per EVSE, so an EVSE without
  a plan (a new session, or a CSMS outage) never draws more than the CSMS has reserved for it.

Limits are always in W. For AC charging needs, the EV's maximum power is
`evMaxCurrent × evMaxVoltage × phases`, where the phase count comes from the EVSE
configuration, capped by the EVSE rating.

## Error codes

Frame-level problems:

| Problem | Error code |
|---|---|
| Not JSON, not an array, wrong number of elements | `RpcFrameworkError` |
| Message id missing, not a string, empty or longer than 36 characters | `RpcFrameworkError` |
| Unknown message type number | `MessageTypeNotSupported` |
| CALL action not a string | `RpcFrameworkError` |
| Payload not a JSON object | `FormatViolation` |

A broken CALL is answered with a CALLERROR that reuses its message id, or `"-1"` when the id
can't be read. A broken CALLRESULT or CALLERROR is never answered.

Payload problems, once the action is known:

| Problem | Error code |
|---|---|
| Required property missing, array too short or too long | `OccurrenceConstraintViolation` |
| Wrong JSON type (for example `"evseId": "one"`) | `TypeConstraintViolation` |
| Unknown enum value, string above `maxLength`, invalid `date-time`, integer out of range | `PropertyConstraintViolation` |
| Unknown property, anything else | `FormatViolation` |

serde reports errors as text only, so the crate maps them by message prefix. Unit tests pin
each mapping.

## Validation details

- Objects reject unknown properties, mirroring `additionalProperties: false`. `customData` is
  the exception: it keeps any vendor-defined keys, as the schemas allow.
- String `maxLength` (counted in characters) and array `minItems` / `maxItems` are enforced by
  the types.
- `date-time` values accept any RFC 3339 offset and are emitted in UTC (`Z`) with at most
  three fractional-second digits, which is OCPP's limit.
- Schema integers are `i32` and schema numbers are `f64`.
- Not enforced by the types: the numeric ranges of `stateOfCharge`, `fullSoC` and `bulkSoC`
  (0–100) and `ePriceLevel` (≥ 0). The schema tests cover them.

## Defaults for EVs without charging needs

EVs without ISO 15118 never send `NotifyEVChargingNeeds`. How their departure time and
energy are estimated will be documented here along with the CSMS session handling.

## Open spec questions

- The `"-1"` message id for CALLERRORs answering an unreadable id should be confirmed
  against OCPP 2.0.1 Part 4 (marked `TODO(spec)` in `crates/ocpp/src/rpc.rs`).
