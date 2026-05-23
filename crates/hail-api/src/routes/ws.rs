//! WebSocket event channel (`GET /api/ws`).
//!
//! Auth middleware runs before the upgrade, then this handler multiplexes
//! app events from [`crate::events::AppEventBus`] plus a per-connection
//! heartbeat. Worker-originated events reach that bus through the SQLite
//! bridge in `events.rs`.

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Router, routing::get};
use chrono::Utc;
use tokio::sync::broadcast;
use tokio::time::{self, Duration};
use url::Url;

use crate::events::{AppEvent, AppEventEnvelope};
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

pub fn router() -> Router<AppState> {
    Router::new().route("/api/ws", get(ws_handler))
}

async fn ws_handler(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !origin_matches_public_url(&headers, &state.config.server.public_url) {
        tracing::debug!("websocket rejected due to missing or foreign Origin");
        return forbidden_origin();
    }

    let events = state.events.subscribe();
    ws.on_upgrade(move |socket| serve_socket(socket, events, user.id, HEARTBEAT_INTERVAL))
}

fn forbidden_origin() -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"origin_forbidden"}"#,
    )
        .into_response()
}

fn origin_matches_public_url(headers: &HeaderMap, public_url: &str) -> bool {
    // `/api/ws` uses ambient session cookies, so require a browser Origin
    // that exactly matches the configured public URL. Non-browser clients
    // without Origin can use the REST API; the WebSocket channel is SPA-only.
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };

    let Ok(origin) = Url::parse(origin) else {
        return false;
    };
    let Ok(public_url) = Url::parse(public_url) else {
        tracing::warn!("configured public_url is not a valid URL; rejecting websocket origin");
        return false;
    };

    same_origin(&origin, &public_url)
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

async fn serve_socket(
    mut socket: WebSocket,
    mut events: broadcast::Receiver<AppEventEnvelope>,
    user_id: i64,
    heartbeat_interval: Duration,
) {
    let mut heartbeat = time::interval(heartbeat_interval);
    heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    // `tokio::time::interval` ticks immediately on first poll; consume
    // that first tick so client-visible heartbeats are actually spaced
    // by `HEARTBEAT_INTERVAL` after connection establishment.
    heartbeat.tick().await;

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                let event = AppEvent::Heartbeat { at: Utc::now() };
                if !send_json(&mut socket, event).await {
                    break;
                }
            }
            result = events.recv() => {
                match result {
                    Ok(envelope) => {
                        if envelope.user_id.is_some_and(|scope| scope != user_id) {
                            continue;
                        }
                        if !send_json(&mut socket, envelope.event).await {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "websocket event receiver lagged");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            inbound = socket.recv() => {
                match inbound {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(bytes))) => {
                        if socket.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        tracing::debug!(error = %err, "websocket receive failed");
                        break;
                    }
                }
            }
        }
    }
}

async fn send_json(socket: &mut WebSocket, event: AppEvent) -> bool {
    match serde_json::to_string(&event) {
        Ok(json) => socket.send(Message::Text(json.into())).await.is_ok(),
        Err(err) => {
            tracing::error!(error = %err, "failed to serialize app event");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_event_json_shape_is_stable() {
        assert_eq!(
            serde_json::to_value(AppEvent::ImboxNew).unwrap(),
            serde_json::json!({ "type": "imbox.new" })
        );
        assert_eq!(
            serde_json::to_value(AppEvent::FeedNew).unwrap(),
            serde_json::json!({ "type": "feed.new" })
        );
        assert_eq!(
            serde_json::to_value(AppEvent::PapertrailNew).unwrap(),
            serde_json::json!({ "type": "papertrail.new" })
        );
        assert_eq!(
            serde_json::to_value(AppEvent::ScreenerPending).unwrap(),
            serde_json::json!({ "type": "screener.pending" })
        );
        assert_eq!(
            serde_json::to_value(AppEvent::ThreadUpdated).unwrap(),
            serde_json::json!({ "type": "thread.updated" })
        );
        assert_eq!(
            serde_json::to_value(AppEvent::ThreadRemoved).unwrap(),
            serde_json::json!({ "type": "thread.removed" })
        );
        assert_eq!(
            serde_json::to_value(AppEvent::BubbleFired).unwrap(),
            serde_json::json!({ "type": "bubble.fired" })
        );
        assert_eq!(
            serde_json::to_value(AppEvent::SendCompleted).unwrap(),
            serde_json::json!({ "type": "send.completed" })
        );
        assert_eq!(
            serde_json::to_value(AppEvent::SendFailed).unwrap(),
            serde_json::json!({ "type": "send.failed" })
        );
    }
}
