//! Fake charging stations for testing the CSMS end to end with paused time.
//!
//! The stations talk to the connection actor through its channels rather than a socket, so
//! `tokio::time::pause` can run hours of charging in no time. Each one answers
//! `SetChargingProfile` like a real station would: a TxDefaultProfile caps every EVSE, a
//! TxProfile replaces it for its transaction and goes away when the transaction ends. After
//! every change, the site model checks that the EVSEs running a transaction are, together,
//! never allowed more than the grid limit at any time from now on.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use ocpp::messages::SetChargingProfileRequest;
use ocpp::types::ChargingProfilePurpose;
use optimizer::StepProfile;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::station_id::StationId;
use crate::{AppState, Clock, Config, connection};

/// How a station answers `SetChargingProfile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Accept,
    Reject,
    /// Never answers, so the CSMS times out.
    Silent,
}

/// What every station follows, for checking the grid limit.
#[derive(Debug)]
pub(crate) struct Model {
    clock: Clock,
    grid_limit_w: f64,
    evse_max_w: f64,
    stations: BTreeMap<String, StationModel>,
    /// Every broken limit, with the time and the total.
    pub violations: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct StationModel {
    pub mode: Option<Mode>,
    pub tx_default_w: Option<f64>,
    pub evses: BTreeMap<i32, EvseModel>,
    /// Every SetChargingProfile received, accepted or not.
    pub received: Vec<SetChargingProfileRequest>,
}

#[derive(Debug, Default)]
pub(crate) struct EvseModel {
    pub transaction: Option<String>,
    pub tx_profile: Option<StepProfile>,
}

impl Model {
    pub fn station(&self, id: &str) -> &StationModel {
        &self.stations[id]
    }

    /// The TxProfile an EVSE follows, if any.
    pub fn tx_profile(&self, station: &str, evse_id: i32) -> Option<&StepProfile> {
        self.stations[station]
            .evses
            .get(&evse_id)?
            .tx_profile
            .as_ref()
    }

    /// Every TxProfile the station received.
    pub fn tx_profiles_received(&self, station: &str) -> usize {
        self.stations[station]
            .received
            .iter()
            .filter(|r| {
                r.charging_profile.charging_profile_purpose == ChargingProfilePurpose::TxProfile
            })
            .count()
    }

    /// The limit an EVSE follows at `t`.
    fn limit_at(&self, station: &StationModel, evse: &EvseModel, t: i64) -> f64 {
        match &evse.tx_profile {
            Some(p) => p.value_at(t),
            None => station.tx_default_w.unwrap_or(self.evse_max_w),
        }
    }

    /// Records a violation if the EVSEs in a transaction may draw more than the grid limit
    /// at some time from now on.
    fn check(&mut self, why: &str) {
        let now = self.clock.now_secs();
        let charging: Vec<(&StationModel, &EvseModel)> = self
            .stations
            .values()
            .flat_map(|s| s.evses.values().map(move |e| (s, e)))
            .filter(|(_, e)| e.transaction.is_some())
            .collect();
        let mut times: Vec<i64> = charging
            .iter()
            .filter_map(|(_, e)| e.tx_profile.as_ref())
            .flat_map(|p| p.breakpoints().collect::<Vec<_>>())
            .filter(|&t| t > now)
            .collect();
        times.push(now);
        for t in times {
            let total: f64 = charging.iter().map(|(s, e)| self.limit_at(s, e, t)).sum();
            if total > self.grid_limit_w + 1e-3 {
                let v = format!("after {why}: {total} W allowed at t={t} (now {now})");
                self.violations.push(v);
            }
        }
    }

    fn evse(&mut self, station: &str, evse_id: i32) -> &mut EvseModel {
        self.stations
            .get_mut(station)
            .expect("station connected")
            .evses
            .entry(evse_id)
            .or_default()
    }

    /// Applies a SetChargingProfile the station accepted.
    fn apply(&mut self, station: &str, request: &SetChargingProfileRequest) {
        let profile = &request.charging_profile;
        let schedule = &profile.charging_schedule[0];
        match profile.charging_profile_purpose {
            ChargingProfilePurpose::TxDefaultProfile => {
                let limit = schedule.charging_schedule_period[0].limit;
                self.stations.get_mut(station).unwrap().tx_default_w = Some(limit);
            }
            ChargingProfilePurpose::TxProfile => {
                let start = schedule.start_schedule.expect("absolute").timestamp();
                let steps = schedule
                    .charging_schedule_period
                    .iter()
                    .map(|p| (i64::from(p.start_period), p.limit));
                let tx = profile.transaction_id.as_ref().map(|t| t.to_string());
                let evse = self.evse(station, request.evse_id);
                assert_eq!(evse.transaction, tx, "TxProfile for another transaction");
                evse.tx_profile = Some(StepProfile::from_steps(start, steps));
            }
            other => panic!("unexpected profile purpose {other:?}"),
        }
        self.check(&format!("{station} accepted a profile"));
    }
}

/// A CSMS with its control loop running.
pub(crate) struct TestSite {
    pub state: Arc<AppState>,
    model: Arc<Mutex<Model>>,
    _control: JoinHandle<()>,
}

impl TestSite {
    pub fn start(config: Config) -> Self {
        let clock = Clock::starting_at_ms(1_790_000_000_000);
        let model = Model {
            clock,
            grid_limit_w: config.site.grid_limit_w,
            evse_max_w: config.site.evse_max_w,
            stations: BTreeMap::new(),
            violations: Vec::new(),
        };
        let state = AppState::with_clock(config, clock);
        let control = crate::spawn_control_loop(state.clone()).unwrap();
        Self {
            state,
            model: Arc::new(Mutex::new(model)),
            _control: control,
        }
    }

    pub fn model(&self) -> MutexGuard<'_, Model> {
        self.model.lock().unwrap()
    }

    pub fn now_secs(&self) -> i64 {
        self.state.clock.now_secs()
    }

    /// Connects a station, boots it and reports its EVSEs (1 to `evses`) as available.
    pub async fn connect(&self, id: &str, evses: i32) -> FakeStation {
        self.model.lock().unwrap().stations.insert(
            id.to_owned(),
            StationModel {
                mode: Some(Mode::Accept),
                ..StationModel::default()
            },
        );
        let (to_csms, incoming) = mpsc::channel(64);
        let (outgoing, from_csms) = mpsc::channel(64);
        let (results_tx, results) = mpsc::channel(64);
        tokio::spawn(connection::serve(
            self.state.clone(),
            StationId::try_from(id).unwrap(),
            incoming,
            outgoing,
        ));
        tokio::spawn(respond(
            id.to_owned(),
            self.model.clone(),
            from_csms,
            to_csms.clone(),
            results_tx,
        ));
        let mut station = FakeStation {
            id: id.to_owned(),
            clock: self.state.clock,
            model: self.model.clone(),
            to_csms,
            results,
            next_id: 0,
        };
        station
            .call(
                "BootNotification",
                json!({
                    "chargingStation": { "model": "Fake", "vendorName": "Test" },
                    "reason": "PowerUp"
                }),
            )
            .await;
        for evse in 1..=evses {
            station.status(evse, "Available").await;
        }
        station
    }
}

/// Answers the CSMS's calls and forwards the answers to the station's own calls.
async fn respond(
    id: String,
    model: Arc<Mutex<Model>>,
    mut from_csms: mpsc::Receiver<String>,
    to_csms: mpsc::Sender<String>,
    results: mpsc::Sender<Value>,
) {
    while let Some(text) = from_csms.recv().await {
        let frame: Value = serde_json::from_str(&text).unwrap();
        if frame[0] != 2 {
            let _ = results.send(frame).await;
            continue;
        }
        assert_eq!(frame[2], "SetChargingProfile", "unexpected call {frame}");
        let request: SetChargingProfileRequest = ocpp::decode(frame[3].clone()).unwrap();
        let answer = {
            let mut model = model.lock().unwrap();
            let station = model.stations.get_mut(&id).unwrap();
            station.received.push(request.clone());
            match station.mode.unwrap_or(Mode::Accept) {
                Mode::Accept => {
                    model.apply(&id, &request);
                    Some("Accepted")
                }
                Mode::Reject => Some("Rejected"),
                Mode::Silent => None,
            }
        };
        if let Some(status) = answer {
            let reply = json!([3, frame[1], { "status": status }]);
            let _ = to_csms.send(reply.to_string()).await;
        }
    }
}

/// One fake station.
pub(crate) struct FakeStation {
    pub id: String,
    clock: Clock,
    model: Arc<Mutex<Model>>,
    to_csms: mpsc::Sender<String>,
    results: mpsc::Receiver<Value>,
    next_id: u32,
}

impl FakeStation {
    pub fn set_mode(&self, mode: Mode) {
        self.model
            .lock()
            .unwrap()
            .stations
            .get_mut(&self.id)
            .unwrap()
            .mode = Some(mode);
    }

    /// Sends a CALL and returns the CALLRESULT payload.
    pub async fn call(&mut self, action: &str, payload: Value) -> Value {
        self.next_id += 1;
        let id = format!("{}-{}", self.id, self.next_id);
        let frame = json!([2, id, action, payload]);
        self.to_csms.send(frame.to_string()).await.unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(5), self.results.recv())
            .await
            .expect("an answer in time")
            .expect("connection open");
        assert_eq!(reply[0], 3, "{action} failed: {reply}");
        assert_eq!(reply[1], id);
        reply[2].clone()
    }

    fn now(&self) -> String {
        self.clock.now().to_string()
    }

    pub async fn status(&mut self, evse: i32, status: &str) {
        let payload = json!({
            "timestamp": self.now(),
            "connectorStatus": status,
            "evseId": evse,
            "connectorId": 1
        });
        self.call("StatusNotification", payload).await;
    }

    /// Starts transaction `tx` on `evse`, at energy register 0.
    pub async fn start(&mut self, tx: &str, evse: i32) {
        {
            let mut model = self.model.lock().unwrap();
            model.evse(&self.id, evse).transaction = Some(tx.to_owned());
            model.check(&format!("{tx} started"));
        }
        self.status(evse, "Occupied").await;
        let payload = json!({
            "eventType": "Started",
            "timestamp": self.now(),
            "triggerReason": "CablePluggedIn",
            "seqNo": 0,
            "transactionInfo": { "transactionId": tx, "chargingState": "EVConnected" },
            "evse": { "id": evse, "connectorId": 1 },
            "meterValue": [self.meter_value(0.0, 0.0)]
        });
        self.call("TransactionEvent", payload).await;
    }

    /// Reports the energy delivered so far and the power drawn, and whether the EV is full.
    pub async fn meter(&mut self, tx: &str, energy_wh: f64, power_w: f64, full: bool) {
        let state = match (full, power_w > 0.0) {
            (true, _) => "SuspendedEV",
            (false, true) => "Charging",
            // Held at 0 W by its limit.
            (false, false) => "SuspendedEVSE",
        };
        let payload = json!({
            "eventType": "Updated",
            "timestamp": self.now(),
            "triggerReason": "MeterValuePeriodic",
            "seqNo": 1,
            "transactionInfo": { "transactionId": tx, "chargingState": state },
            "meterValue": [self.meter_value(energy_wh, power_w)]
        });
        self.call("TransactionEvent", payload).await;
    }

    pub async fn end(&mut self, tx: &str, evse: i32) {
        let payload = json!({
            "eventType": "Ended",
            "timestamp": self.now(),
            "triggerReason": "EVDeparted",
            "seqNo": 2,
            "transactionInfo": { "transactionId": tx, "stoppedReason": "EVDisconnected" }
        });
        self.call("TransactionEvent", payload).await;
        {
            let mut model = self.model.lock().unwrap();
            let e = model.evse(&self.id, evse);
            e.transaction = None;
            e.tx_profile = None;
        }
        self.status(evse, "Available").await;
    }

    /// An ISO 15118 EV on `evse` reports its needs (DC, so the power is taken as is).
    pub async fn needs(&mut self, evse: i32, departure_secs: i64, energy_wh: f64, max_w: f64) {
        let departure = chrono::DateTime::from_timestamp(departure_secs, 0).unwrap();
        let payload = json!({
            "evseId": evse,
            "chargingNeeds": {
                "requestedEnergyTransfer": "DC",
                "departureTime": ocpp::DateTime::from(departure).to_string(),
                "dcChargingParameters": {
                    "evMaxCurrent": 1, "evMaxVoltage": 1,
                    "evMaxPower": max_w as i32, "energyAmount": energy_wh as i32
                }
            }
        });
        self.call("NotifyEVChargingNeeds", payload).await;
    }

    fn meter_value(&self, energy_wh: f64, power_w: f64) -> Value {
        json!({
            "timestamp": self.now(),
            "sampledValue": [
                { "value": energy_wh, "measurand": "Energy.Active.Import.Register" },
                { "value": power_w, "measurand": "Power.Active.Import" }
            ]
        })
    }

    /// The limit `evse` follows right now.
    pub fn limit_now(&self, evse: i32) -> f64 {
        let model = self.model.lock().unwrap();
        let station = &model.stations[&self.id];
        let evse = &station.evses[&evse];
        model.limit_at(station, evse, self.clock.now_secs())
    }
}
