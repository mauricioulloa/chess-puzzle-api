mod download;
pub mod load;
pub mod parse;

use crate::db::{self, meta_keys};
use anyhow::{Context, Result, bail};
use clap::Args;
use std::path::PathBuf;
use std::time::Instant;

pub use load::{Filters, ImportStats};

pub const DEFAULT_SOURCE_URL: &str = "https://database.lichess.org/lichess_db_puzzle.csv.zst";

#[derive(Args, Debug)]
pub struct ImportArgs {
    /// Path of the SQLite database to build
    #[arg(long, default_value = "data/puzzles.db")]
    pub output: PathBuf,

    /// Use a local .csv.zst or .csv instead of downloading the dump
    #[arg(long)]
    pub input: Option<PathBuf>,

    /// Where the downloaded dump is cached between runs
    #[arg(long, default_value = "data/lichess_db_puzzle.csv.zst")]
    pub cache: PathBuf,

    /// Source of the puzzle dump
    #[arg(long, default_value = DEFAULT_SOURCE_URL)]
    pub source_url: String,

    /// Keep only puzzles with at least this Popularity score (-100..100)
    #[arg(long, default_value_t = 90)]
    pub min_popularity: i64,

    /// Keep only puzzles played at least this many times
    #[arg(long, default_value_t = 100)]
    pub min_plays: i64,

    /// Stop after this many accepted puzzles (for quick local runs)
    #[arg(long)]
    pub limit: Option<usize>,

    /// Overwrite the output database if it already exists
    #[arg(long)]
    pub force: bool,

    /// Skip the final VACUUM (faster, but leaves a larger file)
    #[arg(long)]
    pub no_vacuum: bool,

    /// Re-download even if the cached dump is present
    #[arg(long)]
    pub refresh: bool,
}

pub async fn run(args: ImportArgs) -> Result<()> {
    if args.output.exists() {
        if !args.force {
            bail!(
                "{} already exists; pass --force to rebuild it from scratch",
                args.output.display()
            );
        }
        std::fs::remove_file(&args.output)
            .with_context(|| format!("removing {}", args.output.display()))?;
    }

    let source = match &args.input {
        Some(path) => {
            if !path.exists() {
                bail!("{} does not exist", path.display());
            }
            path.clone()
        }
        None => {
            if args.refresh || !args.cache.exists() {
                download::download(&args.source_url, &args.cache).await?;
            } else {
                tracing::info!("using cached dump at {}", args.cache.display());
            }
            args.cache.clone()
        }
    };

    let filters = Filters {
        min_popularity: args.min_popularity,
        min_plays: args.min_plays,
        limit: args.limit,
    };

    tracing::info!(
        "importing from {} (min_popularity={}, min_plays={})",
        source.display(),
        filters.min_popularity,
        filters.min_plays
    );

    let started = Instant::now();
    let mut conn = db::open_for_import(&args.output)?;
    let stats = load::load(&mut conn, &source, filters)?;

    if stats.accepted == 0 {
        bail!("no puzzles survived the filters; loosen --min-popularity or --min-plays");
    }

    load::finalise(&conn, !args.no_vacuum)?;

    let source_label = args
        .input
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| args.source_url.clone());

    for (key, value) in [
        (meta_keys::SCHEMA_VERSION, db::SCHEMA_VERSION.to_string()),
        (meta_keys::SOURCE_URL, source_label),
        (meta_keys::IMPORTED_AT, jiff::Timestamp::now().to_string()),
        (meta_keys::PUZZLE_COUNT, stats.accepted.to_string()),
        (meta_keys::THEME_COUNT, stats.theme_count.to_string()),
        (meta_keys::OPENING_COUNT, stats.opening_count.to_string()),
        (meta_keys::ROWS_READ, stats.rows_read.to_string()),
        (
            meta_keys::MIN_POPULARITY,
            filters.min_popularity.to_string(),
        ),
        (meta_keys::MIN_PLAYS, filters.min_plays.to_string()),
    ] {
        db::set_meta(&conn, key, &value)?;
    }

    drop(conn);
    report(&args, &stats, started.elapsed())?;
    Ok(())
}

fn report(args: &ImportArgs, stats: &ImportStats, elapsed: std::time::Duration) -> Result<()> {
    let bytes = std::fs::metadata(&args.output)
        .map(|m| m.len())
        .unwrap_or(0);
    let kept_pct = if stats.rows_read == 0 {
        0.0
    } else {
        stats.accepted as f64 / stats.rows_read as f64 * 100.0
    };

    println!();
    println!("Import complete in {:.1}s", elapsed.as_secs_f64());
    println!("  source rows        {}", stats.rows_read);
    println!("  puzzles kept       {} ({kept_pct:.1}%)", stats.accepted);
    println!("  themes             {}", stats.theme_count);
    println!("  openings           {}", stats.opening_count);
    println!("  dropped (pop.)     {}", stats.rejected_popularity);
    println!("  dropped (plays)    {}", stats.rejected_plays);
    println!("  dropped (invalid)  {}", stats.rejected_malformed);
    if stats.unparsed_game_urls > 0 {
        println!("  unlinkable games   {}", stats.unparsed_game_urls);
    }
    println!(
        "  database           {} ({:.0} MB)",
        args.output.display(),
        bytes as f64 / 1_048_576.0
    );
    Ok(())
}
