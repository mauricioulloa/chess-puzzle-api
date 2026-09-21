use crate::api::catalog::Catalog;
use crate::api::query::Sampler;
use crate::api::routes;
use crate::auth::keys::KeyStore;
use crate::auth::middleware::AuthState;
use crate::auth::ratelimit::RateLimiter;
use crate::db::pool;
use anyhow::{Context, Result};
use clap::Args;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Path to the puzzle database built by `import`
    #[arg(long, env = "PUZZLES_DB", default_value = "data/puzzles.db")]
    pub db: PathBuf,

    /// Address to listen on
    #[arg(long, env = "BIND_ADDR", default_value = "127.0.0.1:8080")]
    pub bind: String,

    /// Number of pooled read connections
    #[arg(long, env = "POOL_SIZE", default_value_t = 8)]
    pub pool_size: u32,

    /// Path to the read-write database holding API keys
    #[arg(long, env = "API_DB", default_value = "data/api.db")]
    pub api_db: PathBuf,

    /// Requests per minute allowed without an API key
    #[arg(long, env = "ANONYMOUS_LIMIT", default_value_t = 30)]
    pub anonymous_limit: u32,

    /// Believe `X-Forwarded-For`. Only enable this behind a proxy that
    /// overwrites the header, or callers can forge their own client address.
    #[arg(long, env = "TRUST_PROXY_HEADERS")]
    pub trust_proxy_headers: bool,
}

/// How often buffered usage counts are written and stale rate-limit windows
/// are dropped.
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(30);

pub async fn run(args: ServeArgs) -> Result<()> {
    let pool = pool::open_read_only(&args.db, args.pool_size)?;

    let catalog = {
        let conn = pool.get().context("acquiring a connection for startup")?;
        Catalog::load(&conn)?
    };

    if catalog.is_empty() {
        anyhow::bail!(
            "{} has no themes; it looks like an incomplete import",
            args.db.display()
        );
    }

    tracing::info!(
        "loaded {} puzzles across {} themes from {}",
        catalog.puzzle_count,
        catalog.len(),
        args.db.display()
    );

    let store = Arc::new(KeyStore::open(&args.api_db)?);
    let auth = Arc::new(AuthState {
        store: Arc::clone(&store),
        limiter: Arc::new(RateLimiter::new()),
        usage: Mutex::new(HashMap::new()),
        anonymous_limit: args.anonymous_limit,
        trust_proxy_headers: args.trust_proxy_headers,
    });

    if args.trust_proxy_headers {
        tracing::warn!("trusting X-Forwarded-For; only correct behind a proxy that sets it");
    }
    tracing::info!(
        "anonymous callers get {} requests/min; keys carry their own limit",
        args.anonymous_limit
    );

    tokio::spawn(maintenance(Arc::clone(&auth), Arc::clone(&store)));

    let state = Arc::new(Sampler::new(pool, catalog));
    let app = routes::router(state, Arc::clone(&auth));

    let listener = TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    tracing::info!("listening on http://{}", listener.local_addr()?);

    // ConnectInfo is what gives anonymous callers a rate-limit identity.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("serving")?;

    // Whatever accumulated since the last tick would otherwise be lost.
    if let Err(err) = store.flush_usage(&auth.take_usage()) {
        tracing::warn!("could not flush usage on shutdown: {err:#}");
    }
    Ok(())
}

/// Periodically persists usage counts and forgets rate-limit windows for
/// callers that have gone away.
async fn maintenance(auth: Arc<AuthState>, store: Arc<KeyStore>) {
    let mut ticker = tokio::time::interval(MAINTENANCE_INTERVAL);
    ticker.tick().await;
    loop {
        ticker.tick().await;
        if let Err(err) = store.flush_usage(&auth.take_usage()) {
            tracing::warn!("could not flush usage: {err:#}");
        }
        auth.limiter.prune();
    }
}

/// Waits for either interactive interruption or the signal an orchestrator
/// actually sends. Listening only for SIGINT would mean every container
/// restart and every deploy killed the process outright, discarding whatever
/// usage had accumulated since the last flush.
async fn shutdown_signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
        "SIGINT"
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
                "SIGTERM"
            }
            Err(err) => {
                tracing::warn!("cannot listen for SIGTERM: {err}");
                std::future::pending().await
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<&str>();

    let signal = tokio::select! {
        signal = interrupt => signal,
        signal = terminate => signal,
    };
    tracing::info!("received {signal}, shutting down");
}
