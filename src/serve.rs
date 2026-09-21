use crate::api::catalog::Catalog;
use crate::api::query::Sampler;
use crate::api::routes;
use crate::db::pool;
use anyhow::{Context, Result};
use clap::Args;
use std::path::PathBuf;
use std::sync::Arc;
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
}

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

    let state = Arc::new(Sampler::new(pool, catalog));
    let app = routes::router(state);

    let listener = TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    tracing::info!("listening on http://{}", listener.local_addr()?);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving")?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}
