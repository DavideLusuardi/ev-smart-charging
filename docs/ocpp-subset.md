# Supported OCPP 2.0.1 subset

This project implements the part of OCPP 2.0.1 needed for smart charging: stations report
sessions and EV charging needs, and the CSMS answers with charging profiles. The protocol
types live in [`crates/ocpp`](../crates/ocpp) and are validated against the official OCA
JSON schemas in [`specs/ocpp-2.0.1`](../specs/ocpp-2.0.1) (see `crates/ocpp/tests/schema.rs`).

## Transport and framing

OCPP-J over WebSocket. Stations connect to `ws://host:9000/ocpp/{stationId}` and must offer
the subprotocol `ocpp2.0.1`:

- Without it, the CSMS still completes the handshake, with no `Sec-WebSocket-Protocol` in the
  response, and closes the connection at once with close code 1002 (protocol error). A client
  that offered only other subprotocols normally refuses a handshake like that on its own.
- A station id that is empty or longer than 48 characters gets HTTP 400 instead of an
  upgrade.
- A station that connects again under the same id replaces its old connection, which the
  CSMS closes. The old socket is usually half-dead: the station noticed the drop first.

Every WebSocket text message is one frame. Binary messages are ignored.

| Frame | Shape |
|---|---|
| CALL | `[2, messageId, action, payload]` |
| CALLRESULT | `[3, messageId, payload]` |
| CALLERROR | `[4, messageId, errorCode, errorDescription, errorDetails]` |

- `messageId` is a string of 1 to 36 characters. Outgoing ids are UUID v4.
- `payload` and `errorDetails` must be JSON objects.
- A CALLRESULT doesn't name its action, so payloads are decoded into typed messages only once
  the response has been matched to its pending CALL.

## Connection handling

- **Calls from the CSMS** are sent one at a time per connection. Further calls wait in a
  queue until the previous one is answered or times out (30 s by default). The timeout
  starts when a call is sent, not while it waits in the queue. A response that arrives after
  its call timed out is logged and dropped.
- **Calls from a station** are answered in the order they arrive.
- **Boot first is not required.** A station may send any message before
  `BootNotification`: one that reconnects after a network drop carries on without booting
  again, so refusing it would cut off a working station.
- **`BootNotification`** is always accepted, with the configured heartbeat interval (300 s
  by default).
- **`Authorize`** accepts every id token, or only the ones on a configured allowlist
  (`Invalid` otherwise). The token type is not checked.

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

## Sessions

A `TransactionEvent` session is keyed by `transactionId` and planned once its EVSE is known.

- **`Started`** creates the session. Without an `evse` it isn't planned until a later event
  names one. A new session on an EVSE that still has one replaces it: the CSMS missed the
  old one's `Ended`.
- **`Updated`** applies meter values and the charging state. A transaction the CSMS has
  never seen is adopted from that event on, which covers a CSMS restart or a lost `Started`.
- **`Ended`** removes the session. No `ClearChargingProfile` is sent: a station drops a
  TxProfile together with its transaction.
- The response carries `idTokenInfo`, checked like `Authorize`, when the request has an
  `idToken`.

Meter values are read from the most recent `meterValue` of the event, using only whole-EVSE
sampled values (no `phase`), in `Wh`/`kWh` and `W`/`kW` with the unit's `multiplier`:

| Measurand | Use |
|---|---|
| `Energy.Active.Import.Register` (also a value with no measurand) | Delivered energy, counted from the first reading of the transaction |
| `Power.Active.Import` | The power drawn, which teaches the EV's own max power |

An EV that draws clearly less than its limit (by 5 % or 500 W, whichever is more) is
limiting itself, so the planner stops reserving more for it and plans it at that draw plus the
same margin. It is only learned while `chargingState` is `Charging`: an EV that is merely
plugged in draws nothing. `SuspendedEV` marks the EV as full, so it needs nothing more until it
charges again. `SuspendedEVSE` changes nothing: that is the station obeying a 0 W limit.

## Charging needs

`NotifyEVChargingNeeds` gives the session its departure time, energy and max power, and the
CSMS answers `Accepted`:

| Field | Source |
|---|---|
| Departure | `departureTime` |
| Energy | `acChargingParameters.energyAmount`, or `dcChargingParameters.energyAmount` |
| AC max power | `evMaxCurrent × evMaxVoltage × phases`, the phases of `requestedEnergyTransfer` (1, 2 or 3), at most the phases wired to the EVSEs (3 by default) |
| DC max power | `evMaxPower`, or `evMaxCurrent × evMaxVoltage` without it |

The EVSE rating caps the power either way. Needs that arrive before the transaction starts
wait on the EVSE for the next session there, and are dropped if the EVSE reports `Available`
first.

## Charging profile conventions

The CSMS sends two kinds of profile, both with `stackLevel` 0 and `chargingRateUnit = W`:

- **`TxDefaultProfile`**, right after an accepted `BootNotification`: `evseId` 0, id 1,
  `Absolute` from the boot time, one period at the fallback limit (1.4 kW by default). Every
  EVSE of the station stays within that until it gets a plan, and falls back to it if the
  CSMS goes away.
- **`TxProfile`** for each running transaction: `Absolute`, the `transactionId` set, id
  `100 + evseId` so each new profile replaces the one before, `startSchedule` = the planning
  time, and one `chargingSchedulePeriod` per change in the planned limit. The last period is
  0 W at the EV's departure when that falls within the planning horizon; otherwise the last
  planned limit holds.

Limits are always in W, so AC profiles don't depend on the station's phase handling.

## Control loop

One task plans the site and sends the profiles, with the same `Planner` and `dispatch` code
as the offline simulator:

- **When:** every 30 s, and 1 s after an event that changes the plan: a session starts or
  ends, charging needs arrive for a running session, an EV becomes full or charges again, a
  new EVSE is reported, or a station disconnects. The 1 s turns a burst of events into one plan.
- **Input:** each session's needs, reported or assumed. The planner's capacity is the grid
  limit minus the fallback limit of every idle EVSE, which may start drawing it at any moment.
  Sessions on a disconnected station can't be steered: their highest future limit is taken
  off the capacity and they are left out of the plan.
- **Threshold:** a new profile is only sent when, at some point, it differs from the one in
  force by more than 1 kW or 5 % of it, whichever is larger, or when it starts or stops
  charging. If holding back small changes
  would put the site over the limit, every change is sent.
- **Two phases:** first each changed EVSE gets the lower of its new and current profile,
  which only lowers limits, so the total stays within the old total whichever order stations
  accept them in. Once every one is accepted, the EVSEs that go up get their new profile.
  A profile that is lower at some times and higher at others costs both messages; a new
  session usually does, since its plan ends at 0 W, below the fallback.

The CSMS keeps, per EVSE, the profile it believes the station follows, and stays on the safe
side when it can't be sure:

| What happened | What the CSMS assumes |
|---|---|
| `Accepted` | The new profile |
| `Rejected`, or a CALLERROR | The old profile. If this was a decrease, the increases wait for the next tick |
| Timeout, disconnect, or an answer it can't read | The higher of the old and the new profile at every moment, until a later one is accepted |

A station that keeps rejecting decreases therefore holds back increases on the whole site.
Handling it separately, like a disconnected station, is future work.

One short window remains. When an EV throttled below the fallback limit leaves, its EVSE is
back on its TxDefaultProfile before the next plan (about 1 s later) has made room for it. An
empty EVSE draws nothing, so this only matters if a new EV plugs in within that second.

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

EVs without ISO 15118 never send `NotifyEVChargingNeeds`, and EVs that do may leave fields
out. The gaps are filled from site defaults:

| Need | Default |
|---|---|
| Departure | 8 h after the session started |
| Energy | 20 kWh for the whole session |
| Max power | The EVSE rating, until meter values show less |

A default is only a guess, so it rolls forward instead of cutting the EV off:

- An EV still plugged in at its assumed departure is assumed to leave one slot (5 min) later.
- An EV still charging when its assumed energy is nearly delivered is assumed to need at
  least one more slot at its max power.
- Reported energy is trusted until it is delivered, then rolls forward the same way.

## Open spec questions

- The `"-1"` message id for CALLERRORs answering an unreadable id should be confirmed
  against OCPP 2.0.1 Part 4 (marked `TODO(spec)` in `crates/ocpp/src/rpc.rs`).
- The 48-character limit on station ids, and whether their character set should be
  restricted, should be confirmed against Part 4 (marked `TODO(spec)` in
  `crates/csms/src/station_id.rs`).
- Part 4 says to close the connection when no subprotocol is agreed, but as far as we know
  it doesn't name a close code. 1002 (protocol error) is our choice.
- A TxDefaultProfile on `evseId` 0 should limit each EVSE separately, not the station as a
  whole (`crates/csms/src/profiles.rs`).
- A SetChargingProfile whose id already exists on the station should replace that profile
  (`crates/csms/src/profiles.rs`).
- A sampled value without a `measurand` should be `Energy.Active.Import.Register`
  (`crates/csms/src/meter.rs`).
- A TransactionEventResponse should include `idTokenInfo` whenever the request has an
  `idToken` (`crates/csms/src/transactions.rs`).
