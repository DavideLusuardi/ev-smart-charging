//! One actor task per station connection.
//!
//! The actor owns everything about its connection and handles one event at a time, so none
//! of it needs a lock. It exchanges text frames with the socket through two channels, which
//! keeps it independent of the WebSocket library and lets tests drive it directly.

use std::ops::ControlFlow;
use std::sync::Arc;

use ocpp::Frame;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

use crate::AppState;
use crate::handlers;
use crate::station_id::StationId;

/// Registers the station, runs its connection until it ends, then unregisters it.
///
/// `incoming` yields the text frames the station sends and ends when the socket closes;
/// frames for the station go to `outgoing`.
pub(crate) async fn serve(
    state: Arc<AppState>,
    station: StationId,
    incoming: mpsc::Receiver<String>,
    outgoing: mpsc::Sender<String>,
) {
    let registration = state.registry.register(station.clone());
    info!("station connected");
    let connection = Connection { outgoing };
    connection.run(incoming, registration.stop).await;
    state.registry.unregister(&station, registration.generation);
    info!("station disconnected");
}

struct Connection {
    outgoing: mpsc::Sender<String>,
}

impl Connection {
    async fn run(mut self, mut incoming: mpsc::Receiver<String>, mut stop: oneshot::Receiver<()>) {
        loop {
            let flow = tokio::select! {
                text = incoming.recv() => match text {
                    Some(text) => self.on_text(&text).await,
                    None => ControlFlow::Break(()),
                },
                _ = &mut stop => {
                    info!("replaced by a newer connection");
                    ControlFlow::Break(())
                }
            };
            if flow.is_break() {
                break;
            }
        }
    }

    async fn on_text(&mut self, text: &str) -> ControlFlow<()> {
        match Frame::parse(text) {
            Err(e) => {
                warn!(error = %e, "malformed frame");
                match e.reply() {
                    Some(reply) => self.send(reply).await,
                    None => ControlFlow::Continue(()),
                }
            }
            Ok(Frame::Call {
                id,
                action,
                payload,
            }) => {
                debug!(%id, %action, "call from station");
                self.send(handlers::handle(id, &action, payload)).await
            }
            Ok(response) => {
                warn!(id = %response.id(), "response with no call pending, dropped");
                ControlFlow::Continue(())
            }
        }
    }

    /// Queues a frame for the writer; breaks once the socket is gone.
    async fn send(&self, frame: Frame) -> ControlFlow<()> {
        match self.outgoing.send(frame.to_json()).await {
            Ok(()) => ControlFlow::Continue(()),
            Err(_) => ControlFlow::Break(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use serde_json::{Value, json};
    use tokio::task::JoinHandle;

    struct Harness {
        state: Arc<AppState>,
        to_csms: mpsc::Sender<String>,
        from_csms: mpsc::Receiver<String>,
        task: JoinHandle<()>,
    }

    fn start(station: &str) -> Harness {
        start_with(AppState::new(Config::default()), station)
    }

    fn start_with(state: Arc<AppState>, station: &str) -> Harness {
        let (to_csms, incoming) = mpsc::channel(8);
        let (outgoing, from_csms) = mpsc::channel(8);
        let station = StationId::try_from(station).unwrap();
        let task = tokio::spawn(serve(state.clone(), station, incoming, outgoing));
        Harness {
            state,
            to_csms,
            from_csms,
            task,
        }
    }

    impl Harness {
        async fn exchange(&mut self, text: &str) -> Value {
            self.to_csms.send(text.to_owned()).await.unwrap();
            let reply = self.from_csms.recv().await.expect("a reply");
            serde_json::from_str(&reply).unwrap()
        }
    }

    #[tokio::test]
    async fn malformed_frames_get_a_call_error() {
        let mut h = start("CS1");
        let reply = h.exchange("not json").await;
        assert_eq!(reply[0], 4);
        assert_eq!(reply[1], "-1");
        assert_eq!(reply[2], "RpcFrameworkError");

        let reply = h.exchange(r#"[2,"7","Heartbeat",[]]"#).await;
        assert_eq!(reply[1], "7");
        assert_eq!(reply[2], "FormatViolation");
    }

    #[tokio::test]
    async fn unknown_actions_are_not_implemented() {
        let mut h = start("CS1");
        let reply = h.exchange(r#"[2,"1","DataTransfer",{}]"#).await;
        assert_eq!(reply, json!([4, "1", "NotImplemented", reply[3], {}]));
    }

    #[tokio::test]
    async fn stray_responses_are_dropped() {
        let mut h = start("CS1");
        h.to_csms
            .send(r#"[3,"nobody-asked",{}]"#.to_owned())
            .await
            .unwrap();
        // The next call is still answered, and nothing was sent for the stray response.
        let reply = h.exchange(r#"[2,"2","DataTransfer",{}]"#).await;
        assert_eq!(reply[1], "2");
    }

    #[tokio::test]
    async fn closing_the_socket_unregisters_the_station() {
        let h = start("CS1");
        let id = StationId::try_from("CS1").unwrap();
        tokio::task::yield_now().await;
        assert!(h.state.registry.is_connected(&id));

        drop(h.to_csms);
        h.task.await.unwrap();
        assert!(!h.state.registry.is_connected(&id));
    }

    #[tokio::test]
    async fn a_reconnect_replaces_the_old_connection() {
        let old = start("CS1");
        tokio::task::yield_now().await;
        let new = start_with(old.state.clone(), "CS1");
        old.task.await.unwrap();

        tokio::task::yield_now().await;
        let id = StationId::try_from("CS1").unwrap();
        assert!(new.state.registry.is_connected(&id));
    }
}
