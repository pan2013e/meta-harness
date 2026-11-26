use axum::{
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use serde::Deserialize;

use mh_core::WsEvent;

use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientWsCmd {
    SubscribeTrajectory { turn_id: Option<String> },
    SetSession { session_id: Option<String> },
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(q): Query<WsQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, q.session_id))
}

async fn handle_socket(mut socket: WebSocket, state: AppState, mut active_session: Option<String>) {
    let mut rx = state.switch.events_tx.subscribe();
    let mut subscribed_turn: Option<String> = None;

    loop {
        tokio::select! {
            recv_res = rx.recv() => {
                let Ok(ev) = recv_res else {
                    break;
                };
                if !should_forward(&ev, active_session.as_deref(), subscribed_turn.as_deref()) {
                    continue;
                }
                if let Ok(txt) = serde_json::to_string(&ev)
                    && socket.send(Message::Text(txt.into())).await.is_err() {
                        break;
                    }
            }
            ws_msg = socket.recv() => {
                match ws_msg {
                    Some(Ok(Message::Text(txt))) => {
                        if let Ok(cmd) = serde_json::from_str::<ClientWsCmd>(&txt) {
                            match cmd {
                                ClientWsCmd::SubscribeTrajectory { turn_id } => {
                                    subscribed_turn = turn_id;
                                }
                                ClientWsCmd::SetSession { session_id } => {
                                    active_session = session_id;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = socket.send(Message::Pong(p)).await;
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    _ => {}
                }
            }
        }
    }
}

fn should_forward(
    ev: &WsEvent,
    active_session: Option<&str>,
    subscribed_turn: Option<&str>,
) -> bool {
    let ev_session = match ev {
        WsEvent::ChatMessageCreated { message }
        | WsEvent::ChatMessageCompleted { message }
        | WsEvent::SwitchConsultUpdated { message, .. }
        | WsEvent::SwitchActivityPill { message, .. } => Some(message.session_id.as_str()),
        WsEvent::ChatMessageDelta { session_id, .. }
        | WsEvent::SwitchDispatchDecided { session_id, .. }
        | WsEvent::SwitchHopLimitPaused { session_id, .. }
        | WsEvent::MemberStatusChanged { session_id, .. }
        | WsEvent::TrajectoryCounters { session_id, .. }
        | WsEvent::GitBranchUpdated { session_id, .. }
        | WsEvent::GitConflictEscalated { session_id, .. }
        | WsEvent::PermissionResolved { session_id, .. } => Some(session_id.as_str()),
        WsEvent::PermissionRequested { prompt } => Some(prompt.session_id.as_str()),
        WsEvent::TrajectoryEvent {
            session_id,
            turn_id,
            ..
        } => {
            if let Some(sub) = subscribed_turn {
                if sub != turn_id {
                    return false;
                }
            } else {
                // Only send full granular events when drawer has subscribed to a turn
                return false;
            }
            Some(session_id.as_str())
        }
        WsEvent::QuotaUpdated { .. } | WsEvent::MemoryJobCurated { .. } => None,
    };

    ev_session.is_none_or(|actual| active_session == Some(actual))
}
