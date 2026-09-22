use std::{convert::Infallible, time::Duration};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::{
    Stream,
    stream::{self, StreamExt},
};
use sqlx::MySqlPool;
use tokio::time::{Instant, Interval};

use crate::app::AppState;

use super::public_api::{PublicLoginStatus, load_by_id, load_by_public_token};

struct PollState {
    pool: MySqlPool,
    session_id: i64,
    last_updated_at: chrono::NaiveDateTime,
    interval: Interval,
}

pub async fn login_status_events(
    Path(public_token): Path<String>,
    State(state): State<AppState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let row = load_by_public_token(&state.pool, &public_token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if row.is_expired() {
        return Err(StatusCode::GONE);
    }

    let initial_status = row.to_public_status();
    let initial = stream::once(async move { Ok(login_status_event(&initial_status)) });

    let interval = tokio::time::interval_at(
        Instant::now() + Duration::from_secs(1),
        Duration::from_secs(1),
    );
    let updates = stream::unfold(
        PollState {
            pool: state.pool.clone(),
            session_id: row.id,
            last_updated_at: row.updated_at,
            interval,
        },
        |mut poll| async move {
            loop {
                poll.interval.tick().await;

                match load_by_id(&poll.pool, poll.session_id).await {
                    Ok(Some(current)) if current.updated_at != poll.last_updated_at => {
                        poll.last_updated_at = current.updated_at;
                        let status = current.to_public_status();
                        return Some((Ok(login_status_event(&status)), poll));
                    }
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => return None,
                }
            }
        },
    );

    Ok(Sse::new(initial.chain(updates)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    ))
}

fn login_status_event(status: &PublicLoginStatus) -> Event {
    Event::default()
        .event("login_status")
        .data(serde_json::to_string(status).expect("public login status must serialize"))
}
