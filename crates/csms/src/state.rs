//! What the CSMS knows about its stations at runtime.
//!
//! Whether a station is connected right now is the [`Registry`](crate::Registry)'s job; this
//! keeps what the stations reported, which outlives a connection.

use std::collections::BTreeMap;

use ocpp::DateTime;
use ocpp::messages::boot_notification::ChargingStation;
use ocpp::messages::status_notification::ConnectorStatus;

use crate::station_id::StationId;

/// Every station that has connected since the CSMS started.
#[derive(Debug, Default)]
pub struct SiteState {
    stations: BTreeMap<StationId, StationState>,
}

/// One station.
#[derive(Debug, Clone, PartialEq)]
pub struct StationState {
    /// What the station reported in its last `BootNotification`, if it sent one.
    pub info: Option<StationInfo>,
    /// When the CSMS last received a frame from the station.
    pub last_seen: DateTime,
    /// EVSEs by id, as reported in `StatusNotification`s.
    pub evses: BTreeMap<i32, EvseState>,
}

/// A station's identification from `BootNotification`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationInfo {
    pub vendor: String,
    pub model: String,
    pub serial_number: Option<String>,
    pub firmware_version: Option<String>,
}

impl From<&ChargingStation> for StationInfo {
    fn from(station: &ChargingStation) -> Self {
        Self {
            vendor: station.vendor_name.to_string(),
            model: station.model.to_string(),
            serial_number: station.serial_number.as_ref().map(ToString::to_string),
            firmware_version: station.firmware_version.as_ref().map(ToString::to_string),
        }
    }
}

/// One EVSE of a station.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EvseState {
    /// The last reported status of each connector, by connector id.
    pub connectors: BTreeMap<i32, ConnectorStatus>,
}

impl SiteState {
    /// Records that a frame arrived from `id` at `now`, adding the station if it is new.
    pub fn touch(&mut self, id: &StationId, now: DateTime) -> &mut StationState {
        self.stations
            .entry(id.clone())
            .and_modify(|s| s.last_seen = now)
            .or_insert_with(|| StationState {
                info: None,
                last_seen: now,
                evses: BTreeMap::new(),
            })
    }

    /// One station, if it has ever connected.
    pub fn station(&self, id: &StationId) -> Option<&StationState> {
        self.stations.get(id)
    }

    /// All stations, sorted by identity.
    pub fn stations(&self) -> impl Iterator<Item = (&StationId, &StationState)> {
        self.stations.iter()
    }
}
