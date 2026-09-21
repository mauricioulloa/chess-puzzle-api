use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::Write;
use std::path::Path;

/// Streams the dump to disk. Downloads to a `.part` sibling and renames on
/// success, so an interrupted run never leaves a truncated file that looks
/// like a complete cache hit.
pub async fn download(url: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }

    tracing::info!("downloading {url}");
    let response = reqwest::get(url)
        .await
        .with_context(|| format!("requesting {url}"))?;

    if !response.status().is_success() {
        bail!("{url} returned HTTP {}", response.status());
    }

    let progress = match response.content_length() {
        Some(len) => ProgressBar::new(len).with_style(ProgressStyle::with_template(
            "  {bar:40.cyan/blue} {bytes}/{total_bytes} ({bytes_per_sec}, eta {eta})",
        )?),
        None => ProgressBar::no_length()
            .with_style(ProgressStyle::with_template("  {bytes} ({bytes_per_sec})")?),
    };

    let part = dest.with_extension("part");
    let mut file =
        std::fs::File::create(&part).with_context(|| format!("creating {}", part.display()))?;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading response body")?;
        file.write_all(&chunk).context("writing to disk")?;
        progress.inc(chunk.len() as u64);
    }

    file.flush().context("flushing download")?;
    drop(file);
    progress.finish_and_clear();

    std::fs::rename(&part, dest)
        .with_context(|| format!("renaming {} to {}", part.display(), dest.display()))?;
    tracing::info!("saved to {}", dest.display());
    Ok(())
}
