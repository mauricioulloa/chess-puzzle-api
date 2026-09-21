//! Authentication and rate limiting, as one layer.
//!
//! Anonymous callers are allowed but throttled; a key raises the ceiling. An
//! unrecognised key is rejected rather than quietly downgraded to anonymous,
//! because silently ignoring a key the caller believes is working produces
//! confusing 429s later.

use crate::api::errors::ApiError;
use crate::auth::keys::KeyStore;
use crate::auth::ratelimit::{Decision, RateLimiter, Subject};
use crate::usage::{Collector, FilterSample};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

pub struct AuthState {
    pub store: Arc<KeyStore>,
    pub limiter: Arc<RateLimiter>,
    /// Buffered request counts, flushed to disk on a timer.
    pub usage: Mutex<HashMap<i64, u64>>,
    /// Aggregate counters. Separate from `usage` above, which is per-key
    /// and private; these are the ones the public endpoint serves.
    pub stats: Arc<Collector>,
    pub anonymous_limit: u32,
    /// Whether `X-Forwarded-For` may be believed. Off by default: trusting it
    /// when nothing strips it lets anyone reset their own rate limit by
    /// inventing a header.
    pub trust_proxy_headers: bool,
}

impl AuthState {
    pub fn take_usage(&self) -> HashMap<i64, u64> {
        std::mem::take(&mut self.usage.lock().expect("usage lock"))
    }

    fn record(&self, key_id: i64) {
        *self
            .usage
            .lock()
            .expect("usage lock")
            .entry(key_id)
            .or_insert(0) += 1;
    }
}

fn bearer_token(request: &Request) -> Option<&str> {
    let header = request.headers().get(axum::http::header::AUTHORIZATION)?;
    let value = header.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim())
        .filter(|token| !token.is_empty())
}

/// Works out who is calling, for rate limiting.
///
/// The subtlety is which end of `X-Forwarded-For` to believe. A proxy
/// *appends* the address it saw, so the list reads
/// `client, proxy1, proxy2`, and anything to the left of the nearest trusted
/// proxy's entry was supplied by the caller. Taking the leftmost value — the
/// obvious reading — lets anyone mint themselves a fresh rate-limit bucket by
/// sending their own header. So the rightmost entry is used: the one the
/// proxy in front of us wrote and nobody else could have.
///
/// Fly's `Fly-Client-IP` is preferred where present because the proxy sets it
/// wholesale rather than appending, leaving nothing for a caller to prepend.
fn client_ip(request: &Request, trust_proxy_headers: bool) -> IpAddr {
    let peer = || {
        request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip())
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
    };

    if !trust_proxy_headers {
        return peer();
    }

    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };

    if let Some(address) = header("fly-client-ip").and_then(|value| value.trim().parse().ok()) {
        return address;
    }

    if let Some(forwarded) = header("x-forwarded-for")
        && let Some(nearest) = forwarded.rsplit(',').next()
        && let Ok(address) = nearest.trim().parse()
    {
        return address;
    }

    peer()
}

fn apply_headers(response: &mut Response, decision: Decision, authenticated: bool) {
    let headers = response.headers_mut();
    for (name, value) in [
        ("x-ratelimit-limit", decision.limit.to_string()),
        ("x-ratelimit-remaining", decision.remaining.to_string()),
        ("x-ratelimit-reset", decision.reset_after.to_string()),
    ] {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(name, value);
        }
    }
    headers.insert(
        "x-ratelimit-scope",
        HeaderValue::from_static(if authenticated { "key" } else { "anonymous" }),
    );
}

pub async fn enforce(State(auth): State<Arc<AuthState>>, request: Request, next: Next) -> Response {
    let (subject, limit, key_id) = match bearer_token(&request) {
        Some(token) => match auth.store.lookup(token) {
            Ok(Some(record)) => (Subject::Key(record.id), record.rate_limit, Some(record.id)),
            Ok(None) => {
                return ApiError::Unauthorized(
                    "That API key is not valid, or it has been revoked.".to_string(),
                )
                .into_response();
            }
            Err(err) => return ApiError::Internal(err).into_response(),
        },
        None => {
            let ip = client_ip(&request, auth.trust_proxy_headers);
            (Subject::Ip(ip), auth.anonymous_limit, None)
        }
    };

    let decision = auth.limiter.check(subject, limit);
    if !decision.allowed {
        let mut response = ApiError::RateLimited {
            retry_after: decision.reset_after,
            limit: decision.limit,
        }
        .into_response();
        apply_headers(&mut response, decision, key_id.is_some());
        // A throttled request is still traffic worth seeing in the numbers.
        let endpoint = request
            .extensions()
            .get::<axum::extract::MatchedPath>()
            .map(|matched| matched.as_str().to_string())
            .unwrap_or_else(|| request.uri().path().to_string());
        auth.stats
            .record_request(&endpoint, response.status().as_u16(), key_id.is_some());
        return response;
    }

    if let Some(id) = key_id {
        auth.record(id);
    }

    // MatchedPath is the route template, not the concrete URL, so the
    // counters never accumulate a row per puzzle id.
    let endpoint = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|matched| matched.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());

    let mut response = next.run(request).await;
    apply_headers(&mut response, decision, key_id.is_some());

    auth.stats
        .record_request(&endpoint, response.status().as_u16(), key_id.is_some());
    // Handlers that take filters describe them through the response, so each
    // one does not need its own path to the collector.
    if let Some(sample) = response.extensions().get::<FilterSample>() {
        auth.stats.record_filters(sample);
    }
    response
}
