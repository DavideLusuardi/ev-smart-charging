//! The WebSocket endpoint: the subprotocol check and the glue between the socket and the
//! connection actor.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{Instrument, debug, info_span, warn};

use crate::station_id::StationId;
use crate::{AppState, SUBPROTOCOL, connection};

/// Frames buffered in each direction between the socket and the actor.
const FRAME_BUFFER: usize = 16;
/// How long to wait for the Close frame to go out once a connection ends.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// `GET /ocpp/{station_id}`: upgrades to a WebSocket running OCPP 2.0.1.
pub(crate) async fn upgrade(
    Path(station): Path<String>,
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> Response {
    let station = match StationId::try_from(station) {
        Ok(station) => station,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let ws = ws.protocols([SUBPROTOCOL]);
    if ws.selected_protocol().is_none() {
        let offered: Vec<_> = ws
            .requested_protocols()
            .filter_map(|p| p.to_str().ok())
            .collect();
        warn!(%station, ?offered, "rejecting a connection without the {SUBPROTOCOL} subprotocol");
        // OCPP-J: complete the handshake without a subprotocol, then close right away.
        return ws.on_upgrade(reject);
    }
    let span = info_span!("station", id = %station);
    ws.on_upgrade(move |socket| run_socket(state, station, socket).instrument(span))
}

async fn reject(mut socket: WebSocket) {
    let frame = CloseFrame {
        code: close_code::PROTOCOL,
        reason: format!("the {SUBPROTOCOL} subprotocol is required").into(),
    };
    let _ = socket.send(Message::Close(Some(frame))).await;
}

/// Runs one station connection: a reader and a writer task around the actor.
async fn run_socket(state: Arc<AppState>, station: StationId, socket: WebSocket) {
    let (sink, stream) = socket.split();
    let (incoming_tx, incoming) = mpsc::channel(FRAME_BUFFER);
    let (outgoing, outgoing_rx) = mpsc::channel(FRAME_BUFFER);
    let reader = tokio::spawn(read_frames(stream, incoming_tx).in_current_span());
    let writer = tokio::spawn(write_frames(sink, outgoing_rx).in_current_span());

    connection::serve(state, station, incoming, outgoing).await;

    // The actor dropped its sender, so the writer sends a Close frame and ends. The reader
    // may be stuck on a half-dead socket, so it is stopped rather than awaited.
    if tokio::time::timeout(CLOSE_TIMEOUT, writer).await.is_err() {
        debug!("timed out closing the socket");
    }
    reader.abort();
}

async fn read_frames(mut stream: SplitStream<WebSocket>, frames: mpsc::Sender<String>) {
    while let Some(message) = stream.next().await {
        match message {
            Ok(Message::Text(text)) => {
                if frames.send(text.as_str().to_owned()).await.is_err() {
                    return;
                }
            }
            Ok(Message::Binary(_)) => warn!("ignoring a binary message: OCPP-J uses text"),
            Ok(Message::Ping(_) | Message::Pong(_)) => {}
            Ok(Message::Close(frame)) => {
                debug!(?frame, "station closed the socket");
                return;
            }
            Err(e) => {
                debug!(error = %e, "socket error");
                return;
            }
        }
    }
}

async fn write_frames(mut sink: SplitSink<WebSocket, Message>, mut frames: mpsc::Receiver<String>) {
    while let Some(text) = frames.recv().await {
        if let Err(e) = sink.send(Message::Text(text.into())).await {
            debug!(error = %e, "socket error");
            return;
        }
    }
    let _ = sink.close().await;
}
