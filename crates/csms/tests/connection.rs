//! The OCPP endpoint over real WebSockets: the CSMS runs in-process and the tests act as
//! charging stations.

use std::future::IntoFuture;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use csms::{AppState, Config, StationId};
use futures_util::{SinkExt, StreamExt};
use ocpp::messages::clear_charging_profile::ClearChargingProfileStatus;
use ocpp::messages::status_notification::ConnectorStatus;
use ocpp::messages::{ClearChargingProfile, ClearChargingProfileRequest};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::error::{ProtocolError, SubProtocolError};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const PROTOCOL_HEADER: &str = "sec-websocket-protocol";

async fn start_csms(config: Config) -> (SocketAddr, Arc<AppState>) {
    let state = AppState::new(config);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, csms::router(state.clone())).into_future());
    (addr, state)
}

/// Opens a WebSocket to `/ocpp/{station}`, offering `protocol` if given. Returns the socket
/// and the subprotocol the server picked.
async fn connect(
    addr: SocketAddr,
    station: &str,
    protocol: Option<&str>,
) -> Result<(Socket, Option<String>), tungstenite::Error> {
    let mut request = format!("ws://{addr}/ocpp/{station}")
        .into_client_request()
        .unwrap();
    if let Some(protocol) = protocol {
        request
            .headers_mut()
            .insert(PROTOCOL_HEADER, HeaderValue::from_str(protocol).unwrap());
    }
    let (socket, response) = tokio_tungstenite::connect_async(request).await?;
    let selected = response
        .headers()
        .get(PROTOCOL_HEADER)
        .map(|v| v.to_str().unwrap().to_owned());
    Ok((socket, selected))
}

async fn connect_station(addr: SocketAddr, station: &str) -> Socket {
    let (socket, selected) = connect(addr, station, Some("ocpp2.0.1")).await.unwrap();
    assert_eq!(selected.as_deref(), Some("ocpp2.0.1"));
    socket
}

/// Sends a text frame and returns the next frame received, parsed as JSON.
async fn exchange(socket: &mut Socket, text: &str) -> Value {
    socket.send(Message::text(text)).await.unwrap();
    recv_json(socket).await
}

async fn send_json(socket: &mut Socket, frame: Value) {
    socket.send(Message::text(frame.to_string())).await.unwrap();
}

/// Polls `condition` until it holds, for at most a second.
async fn wait_until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("condition not met in time");
}

async fn recv_json(socket: &mut Socket) -> Value {
    loop {
        match socket.next().await.expect("socket open").unwrap() {
            Message::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("expected a text frame, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn accepts_the_ocpp_subprotocol_and_answers_calls() {
    let (addr, _state) = start_csms(Config::default()).await;
    let mut socket = connect_station(addr, "CS1").await;

    let reply = exchange(&mut socket, r#"[2,"1","DataTransfer",{}]"#).await;
    assert_eq!(reply[0], 4);
    assert_eq!(reply[1], "1");
    assert_eq!(reply[2], "NotImplemented");

    let reply = exchange(&mut socket, "[2,").await;
    assert_eq!(reply[1], "-1");
    assert_eq!(reply[2], "RpcFrameworkError");
}

#[tokio::test]
async fn closes_connections_without_the_subprotocol() {
    let (addr, _state) = start_csms(Config::default()).await;

    // The handshake completes without a subprotocol, then the CSMS closes at once.
    let (mut socket, selected) = connect(addr, "CS1", None).await.unwrap();
    assert_eq!(selected, None);
    match socket.next().await {
        Some(Ok(Message::Close(Some(frame)))) => assert_eq!(frame.code, CloseCode::Protocol),
        other => panic!("expected a close frame, got {other:?}"),
    }
}

#[tokio::test]
async fn refuses_other_ocpp_versions() {
    let (addr, _state) = start_csms(Config::default()).await;

    // The CSMS answers without a subprotocol, so a client that offered one refuses the
    // handshake.
    match connect(addr, "CS1", Some("ocpp1.6")).await {
        Err(tungstenite::Error::Protocol(ProtocolError::SecWebSocketSubProtocolError(
            SubProtocolError::NoSubProtocol,
        ))) => {}
        other => panic!("expected the handshake to fail, got {other:?}"),
    }
}

#[tokio::test]
async fn rejects_invalid_station_ids() {
    let (addr, _state) = start_csms(Config::default()).await;
    let long_id = "a".repeat(49);
    match connect(addr, &long_id, Some("ocpp2.0.1")).await {
        Err(tungstenite::Error::Http(response)) => {
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
        other => panic!("expected HTTP 400, got {other:?}"),
    }
}

#[tokio::test]
async fn boot_heartbeat_and_status_update_the_site_state() {
    let (addr, state) = start_csms(Config::default()).await;
    let id = StationId::try_from("CS1").unwrap();
    let mut socket = connect_station(addr, "CS1").await;

    let boot = json!([2, "1", "BootNotification", {
        "chargingStation": { "model": "Wallbox 22", "vendorName": "Acme" },
        "reason": "PowerUp"
    }]);
    send_json(&mut socket, boot).await;
    let reply = recv_json(&mut socket).await;
    assert_eq!(reply[0], 3);
    assert_eq!(reply[1], "1");
    assert_eq!(reply[2]["status"], "Accepted");
    assert_eq!(reply[2]["interval"], 300);
    assert!(state.registry.is_connected(&id));

    // Right after boot, every EVSE is capped at the fallback limit until it gets a plan.
    let call = recv_json(&mut socket).await;
    assert_eq!(call[0], 2);
    assert_eq!(call[2], "SetChargingProfile");
    assert_eq!(call[3]["evseId"], 0);
    let profile = &call[3]["chargingProfile"];
    assert_eq!(profile["chargingProfilePurpose"], "TxDefaultProfile");
    assert_eq!(
        profile["chargingSchedule"][0]["chargingSchedulePeriod"],
        json!([{ "startPeriod": 0, "limit": 1400.0 }])
    );
    send_json(&mut socket, json!([3, call[1], { "status": "Accepted" }])).await;

    let reply = exchange(&mut socket, r#"[2,"2","Heartbeat",{}]"#).await;
    assert!(reply[2]["currentTime"].is_string());

    for (evse, status) in [(1, "Available"), (2, "Occupied")] {
        let frame = json!([2, format!("s{evse}"), "StatusNotification", {
            "timestamp": "2026-09-30T08:00:00Z",
            "connectorStatus": status,
            "evseId": evse,
            "connectorId": 1
        }]);
        send_json(&mut socket, frame).await;
        assert_eq!(
            recv_json(&mut socket).await,
            json!([3, format!("s{evse}"), {}])
        );
    }

    {
        let site = state.site();
        let station = site.station(&id).expect("station known");
        let info = station.info.as_ref().expect("booted");
        assert_eq!(
            (info.vendor.as_str(), info.model.as_str()),
            ("Acme", "Wallbox 22")
        );
        assert_eq!(station.evses[&1].connectors[&1], ConnectorStatus::Available);
        assert_eq!(station.evses[&2].connectors[&1], ConnectorStatus::Occupied);
    }

    // Closing the socket unregisters the station; what it reported stays.
    socket.close(None).await.unwrap();
    wait_until(|| !state.registry.is_connected(&id)).await;
    assert!(state.site().station(&id).is_some());
}

#[tokio::test]
async fn csms_calls_reach_the_station_one_at_a_time() {
    let (addr, state) = start_csms(Config::default()).await;
    let id = StationId::try_from("CS1").unwrap();
    let mut socket = connect_station(addr, "CS1").await;
    wait_until(|| state.registry.is_connected(&id)).await;
    let handle = state.registry.get(&id).unwrap();

    let calls: Vec<_> = (1..=2)
        .map(|profile_id| {
            let handle = handle.clone();
            tokio::spawn(async move {
                let request = ClearChargingProfileRequest {
                    custom_data: None,
                    charging_profile_id: Some(profile_id),
                    charging_profile_criteria: None,
                };
                handle.call::<ClearChargingProfile>(&request).await
            })
        })
        .collect();

    let first = recv_json(&mut socket).await;
    assert_eq!(first[0], 2);
    assert_eq!(first[2], "ClearChargingProfile");
    // The second call waits until the first is answered.
    assert!(
        tokio::time::timeout(Duration::from_millis(200), socket.next())
            .await
            .is_err()
    );
    send_json(&mut socket, json!([3, first[1], { "status": "Accepted" }])).await;

    let second = recv_json(&mut socket).await;
    assert_eq!(second[2], "ClearChargingProfile");
    assert_ne!(second[1], first[1]);
    assert_ne!(second[3], first[3]);
    send_json(&mut socket, json!([3, second[1], { "status": "Unknown" }])).await;

    // Each caller gets the answer to its own call.
    let first_profile = first[3]["chargingProfileId"].as_i64().unwrap();
    for (profile_id, call) in (1..).zip(calls) {
        let expected = if profile_id == first_profile {
            ClearChargingProfileStatus::Accepted
        } else {
            ClearChargingProfileStatus::Unknown
        };
        assert_eq!(call.await.unwrap().unwrap().status, expected);
    }
}

#[tokio::test]
async fn a_reconnecting_station_replaces_its_old_connection() {
    let (addr, state) = start_csms(Config::default()).await;
    let id = StationId::try_from("CS1").unwrap();
    let mut old = connect_station(addr, "CS1").await;
    wait_until(|| state.registry.is_connected(&id)).await;

    let mut new = connect_station(addr, "CS1").await;
    loop {
        match old.next().await {
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
            Some(Ok(_)) => {}
        }
    }

    let reply = exchange(&mut new, r#"[2,"1","Heartbeat",{}]"#).await;
    assert_eq!(reply[0], 3);
    assert!(state.registry.is_connected(&id));
}
