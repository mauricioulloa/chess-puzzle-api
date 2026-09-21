//! The `keys` subcommand.
//!
//! Keys are minted by hand rather than self-service: someone opens an issue,
//! you run one command, you send them the line it prints.

use crate::auth::keys::KeyStore;
use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct KeysArgs {
    /// Path to the read-write database holding API keys
    #[arg(long, env = "API_DB", default_value = "data/api.db")]
    pub api_db: PathBuf,

    #[command(subcommand)]
    pub command: KeysCommand,
}

#[derive(Subcommand, Debug)]
pub enum KeysCommand {
    /// Mint a key. The key is printed once and cannot be recovered.
    Create {
        /// Who the key is for, so you can identify it later
        #[arg(long)]
        label: String,

        /// Requests per minute this key allows
        #[arg(long, default_value_t = 600)]
        rate_limit: u32,
    },
    /// List issued keys, without their secrets
    List,
    /// Revoke every active key carrying a label
    Revoke {
        #[arg(long)]
        label: String,
    },
}

pub fn run(args: KeysArgs) -> Result<()> {
    let store = KeyStore::open(&args.api_db)?;

    match args.command {
        KeysCommand::Create { label, rate_limit } => {
            let key = store.create(&label, rate_limit)?;
            println!("Key for {label} ({rate_limit} requests/min):");
            println!();
            println!("  {key}");
            println!();
            println!("Only the hash is stored, so this is the only time it can be read.");
            println!("Send it over something private, and issue a new one if it leaks.");
        }
        KeysCommand::List => {
            let keys = store.list()?;
            if keys.is_empty() {
                println!("No keys issued yet.");
                return Ok(());
            }
            println!(
                "{:>3}  {:<28} {:>8}  {:>10}  {:<9} created",
                "id", "label", "req/min", "requests", "state"
            );
            for key in keys {
                println!(
                    "{:>3}  {:<28} {:>8}  {:>10}  {:<9} {}",
                    key.id,
                    key.label,
                    key.rate_limit,
                    key.request_count,
                    if key.revoked { "revoked" } else { "active" },
                    key.created_at,
                );
            }
        }
        KeysCommand::Revoke { label } => match store.revoke(&label)? {
            0 => println!("No active key labelled {label}."),
            n => println!("Revoked {n} key(s) labelled {label}. Effective immediately."),
        },
    }
    Ok(())
}
