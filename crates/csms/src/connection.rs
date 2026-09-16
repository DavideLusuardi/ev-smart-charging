//! One actor task per station connection.
//!
//! The actor owns everything about its connection and handles one event at a time, so none
//! of it needs a lock. It exchanges text frames with the socket through two channels, which
//! keeps it independent of the WebSocket library and lets tests drive it directly.
//!
//! OCPP-J allows one outstanding CALL per direction. For CALLs to the station that is a
//! single pending slot: the actor only takes the next queued call once the slot is empty,
//! and a timer clears the slot if the station never answers.

use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;

use ocpp::{DateTime, Frame, MessageId};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, sleep_until};
use tracing::{debug, info, warn};

use crate::AppState;
use crate::handle::{CallError, OutboundCall, StationHandle};
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
    let (calls_tx, calls) = mpsc::channel(state.config.call_queue_len);
    let registration = state
        .registry
        .register(station.clone(), StationHandle::new(calls_tx));
    info!("station connected");
    let connection = Connection {
        state: state.clone(),
        station: station.clone(),
        outgoing,
        calls,
        pending: None,
        call_timeout: state.config.call_timeout,
    };
    connection.run(incoming, registration.stop).await;
    state.registry.unregister(&station, registration.generation);
    info!("station disconnected");
}

struct Connection {
    state: Arc<AppState>,
    station: StationId,
    outgoing: mpsc::Sender<String>,
    /// CALLs waiting to be sent to the station.
    calls: mpsc::Receiver<OutboundCall>,
    /// The CALL sent to the station and not answered yet.
    pending: Option<Pending>,
    call_timeout: Duration,
}

struct Pending {
    id: MessageId,
    action: &'static str,
    reply: oneshot::Sender<Result<Value, CallError>>,
    deadline: Instant,
}

impl Connection {
    /// Runs until the socket closes or a newer connection replaces this one. Dropping the
    /// connection then fails the pending and queued calls with `Disconnected`.
    async fn run(mut self, mut incoming: mpsc::Receiver<String>, mut stop: oneshot::Receiver<()>) {
        loop {
            let deadline = self.pending.as_ref().map(|p| p.deadline);
            let flow = tokio::select! {
                text = incoming.recv() => match text {
                    Some(text) => self.on_text(&text).await,
                    None => ControlFlow::Break(()),
                },
                Some(call) = self.calls.recv(), if self.pending.is_none() => {
                    self.start_call(call).await
                }
                () = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                    self.time_out();
                    ControlFlow::Continue(())
                }
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
        // Any frame, even a broken one, shows the station is alive.
        let now = DateTime::now();
        self.state.site().touch(&self.station, now);
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
                // Handlers run inline on this task, so a handler must never wait for a CALL
                // on its own connection: the response could only be read by this task.
                let reply = {
                    let mut site = self.state.site();
                    // Same timestamp as above; `touch` just hands back the station.
                    let station = site.touch(&self.station, now);
                    handlers::handle(&self.state.config, station, id, &action, payload)
                };
                self.send(reply).await
            }
            Ok(response) => {
                self.on_response(response);
                ControlFlow::Continue(())
            }
        }
    }

    async fn start_call(&mut self, call: OutboundCall) -> ControlFlow<()> {
        if call.reply.is_closed() {
            // The caller stopped waiting while the call was queued.
            return ControlFlow::Continue(());
        }
        let id = MessageId::new_v4();
        debug!(%id, action = call.action, "call to station");
        let frame = Frame::Call {
            id: id.clone(),
            action: call.action.to_owned(),
            payload: call.payload,
        };
        self.pending = Some(Pending {
            id,
            action: call.action,
            reply: call.reply,
            deadline: Instant::now() + self.call_timeout,
        });
        self.send(frame).await
    }

    fn on_response(&mut self, response: Frame) {
        let Some(pending) = self.pending.take_if(|p| p.id == *response.id()) else {
            // Most likely the answer to a call that already timed out.
            warn!(id = %response.id(), "response with no matching call, dropped");
            return;
        };
        let result = match response {
            Frame::CallResult { payload, .. } => Ok(payload),
            Frame::CallError {
                code, description, ..
            } => Err(CallError::Rejected { code, description }),
            Frame::Call { .. } => unreachable!("CALLs are handled by on_text"),
        };
        let _ = pending.reply.send(result);
    }

    fn time_out(&mut self) {
        if let Some(pending) = self.pending.take() {
            warn!(id = %pending.id, action = pending.action, "station did not answer in time");
            let _ = pending.reply.send(Err(CallError::Timeout));
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
    use ocpp::ErrorCode;
    use ocpp::messages::clear_charging_profile::ClearChargingProfileStatus;
    use ocpp::messages::{
        ClearChargingProfile, ClearChargingProfileRequest, ClearChargingProfileResponse,
    };
    use serde_json::json;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;

    type CallTask = JoinHandle<Result<ClearChargingProfileResponse, CallError>>;

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

    fn clear(profile_id: i32) -> ClearChargingProfileRequest {
        ClearChargingProfileRequest {
            custom_data: None,
            charging_profile_id: Some(profile_id),
            charging_profile_criteria: None,
        }
    }

    impl Harness {
        async fn send(&self, text: &str) {
            self.to_csms.send(text.to_owned()).await.unwrap();
        }

        async fn recv(&mut self) -> Value {
            let text = self.from_csms.recv().await.expect("a frame");
            serde_json::from_str(&text).unwrap()
        }

        async fn exchange(&mut self, text: &str) -> Value {
            self.send(text).await;
            self.recv().await
        }

        async fn handle(&self) -> StationHandle {
            tokio::task::yield_now().await;
            let id = StationId::try_from("CS1").unwrap();
            self.state.registry.get(&id).expect("station registered")
        }

        /// Starts a `ClearChargingProfile` call and lets it reach the connection's queue.
        async fn spawn_call(&self, profile_id: i32) -> CallTask {
            let handle = self.handle().await;
            let task = tokio::spawn(async move {
                handle
                    .call::<ClearChargingProfile>(&clear(profile_id))
                    .await
            });
            tokio::task::yield_now().await;
            task
        }

        /// Receives the next CALL, checks it is for `profile_id`, and returns its id.
        async fn recv_call(&mut self, profile_id: i32) -> String {
            let frame = self.recv().await;
            assert_eq!(frame[0], 2);
            assert_eq!(frame[2], "ClearChargingProfile");
            assert_eq!(frame[3], json!({ "chargingProfileId": profile_id }));
            frame[1].as_str().unwrap().to_owned()
        }

        async fn accept(&self, id: &str) {
            self.send(&format!(r#"[3,"{id}",{{"status":"Accepted"}}]"#))
                .await;
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

    #[tokio::test]
    async fn a_call_result_completes_the_call() {
        let mut h = start("CS1");
        let call = h.spawn_call(1).await;
        let id = h.recv_call(1).await;
        h.accept(&id).await;
        let response = call.await.unwrap().unwrap();
        assert_eq!(response.status, ClearChargingProfileStatus::Accepted);
    }

    #[tokio::test(start_paused = true)]
    async fn only_one_call_is_in_flight() {
        let mut h = start("CS1");
        let first = h.spawn_call(1).await;
        let second = h.spawn_call(2).await;

        let id = h.recv_call(1).await;
        // Nothing else goes out while the first call is unanswered.
        assert!(
            timeout(Duration::from_secs(10), h.from_csms.recv())
                .await
                .is_err()
        );
        h.accept(&id).await;
        assert!(first.await.unwrap().is_ok());

        let id = h.recv_call(2).await;
        h.accept(&id).await;
        assert!(second.await.unwrap().is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn an_unanswered_call_times_out_and_the_next_one_goes_out() {
        let mut h = start("CS1");
        let first = h.spawn_call(1).await;
        let second = h.spawn_call(2).await;

        let first_id = h.recv_call(1).await;
        let sent_at = Instant::now();
        assert_eq!(first.await.unwrap(), Err(CallError::Timeout));
        assert_eq!(sent_at.elapsed(), Config::default().call_timeout);

        let second_id = h.recv_call(2).await;
        // The late answer to the first call is dropped.
        h.accept(&first_id).await;
        h.accept(&second_id).await;
        assert!(second.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn call_errors_and_invalid_results_fail_the_call() {
        let mut h = start("CS1");
        let call = h.spawn_call(1).await;
        let id = h.recv_call(1).await;
        h.send(&format!(
            r#"[4,"{id}","NotSupported","no profiles here",{{}}]"#
        ))
        .await;
        assert_eq!(
            call.await.unwrap(),
            Err(CallError::Rejected {
                code: ErrorCode::NotSupported,
                description: "no profiles here".into(),
            })
        );

        let call = h.spawn_call(2).await;
        let id = h.recv_call(2).await;
        h.send(&format!(r#"[3,"{id}",{{"status":"Maybe"}}]"#)).await;
        assert!(matches!(
            call.await.unwrap(),
            Err(CallError::Decode(e)) if e.code == ErrorCode::PropertyConstraintViolation
        ));
    }

    #[tokio::test]
    async fn calls_fail_when_the_station_disconnects() {
        let mut h = start("CS1");
        let handle = h.handle().await;
        let in_flight = h.spawn_call(1).await;
        let queued = h.spawn_call(2).await;
        h.recv_call(1).await;

        drop(h.to_csms);
        h.task.await.unwrap();
        assert_eq!(in_flight.await.unwrap(), Err(CallError::Disconnected));
        assert_eq!(queued.await.unwrap(), Err(CallError::Disconnected));
        assert_eq!(
            handle.call::<ClearChargingProfile>(&clear(3)).await,
            Err(CallError::Disconnected)
        );
    }
}
