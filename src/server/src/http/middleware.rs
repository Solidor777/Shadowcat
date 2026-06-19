use std::sync::atomic::Ordering;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};

use crate::http::AppState;

/// While uninitialized, funnel everything except the setup API, the setup page,
/// and static assets to `/setup.html`. Once an admin exists (cached flag), pass
/// through. Coupling: `/api/setup` flips `initialized` after creating the admin.
pub async fn init_gate(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if state.initialized.load(Ordering::Relaxed) {
        return next.run(req).await;
    }
    // Exact-match allowlist (no suffix matching, which would leak any future
    // route ending in .js/.css): the first-run setup API, health, and the
    // static assets the setup page itself loads.
    let path = req.uri().path();
    let allowed = matches!(
        path,
        "/api/setup" | "/api/config" | "/setup.html" | "/auth.js" | "/styles.css" | "/health"
    );
    if allowed {
        next.run(req).await
    } else {
        Redirect::to("/setup.html").into_response()
    }
}
