//! The OCPP endpoint over real WebSockets: the CSMS runs in-process and the tests act as
//! charging stations.

use std::future::IntoFuture;
use std::net::SocketAddr;
use std::sync::Arc;

use csms::{AppState, Config};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
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
