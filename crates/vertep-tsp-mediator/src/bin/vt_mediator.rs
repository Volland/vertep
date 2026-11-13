//! `vt-mediator`: a TSP mailbox mediator.
//!
//! ```text
//! vt-mediator serve   [--config FILE]   run the mediator (HTTP, WebSocket, expiry sweep)
//! vt-mediator did     [--config FILE]   print the mediator's DID document (creating it)
//! vt-mediator compact [--config FILE]   rewrite the mailbox store with live messages only
//!                                       (offline: stop the server first)
//! vt-mediator revoke  [--config FILE] VID   revoke a VID's mailbox (offline)
//! ```
//!
//! Configuration comes from the TOML file and `VT_MEDIATOR_*` environment variables; see
//! `vertep_tsp_mediator::config`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use vertep_tsp_mediator::config::MediatorConfig;
use vertep_tsp_mediator::server;
use vertep_tsp_mediator::store::MailboxStore;

fn usage() -> ExitCode {
    eprintln!("usage: vt-mediator <serve|did|compact|revoke VID> [--config FILE]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(cmd) = args.next() else {
        return usage();
    };
    let mut config_path: Option<PathBuf> = None;
    let mut rest = Vec::new();
    while let Some(a) = args.next() {
        if a == "--config" {
            config_path = args.next().map(PathBuf::from);
        } else {
            rest.push(a);
        }
    }
    let config = match MediatorConfig::load(config_path.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let result = match cmd.as_str() {
        "serve" => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
            .and_then(|rt| {
                rt.block_on(server::serve(config))
                    .map_err(|e| e.to_string())
            }),
        "did" => server::build(&config)
            .map_err(|e| e.to_string())
            .and_then(|m| m.document().map_err(|e| e.to_string()))
            .map(|doc| println!("{}", doc.to_json())),
        "compact" => compact(&config),
        "revoke" => match rest.first() {
            Some(vid) => server::build(&config)
                .map_err(|e| e.to_string())
                .and_then(|m| m.revoke(vid).map_err(|e| e.to_string())),
            None => return usage(),
        },
        _ => return usage(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vt-mediator: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Copies live mailbox state into a fresh store, then swaps the directories.
fn compact(config: &MediatorConfig) -> Result<(), String> {
    let dir = config.mailbox_dir();
    let fresh = dir.with_extension("compact");
    let old = dir.with_extension("old");
    let _ = std::fs::remove_dir_all(&fresh);
    {
        let current = vertep_tiramemsu::TiramemsuBackend::open(&dir, config.shards)
            .map_err(|e| e.to_string())?;
        let store = MailboxStore::open(Arc::new(current)).map_err(|e| e.to_string())?;
        let target = vertep_tiramemsu::TiramemsuBackend::open(&fresh, config.shards)
            .map_err(|e| e.to_string())?;
        store
            .compact_into(Arc::new(target))
            .map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(&dir, &old).map_err(|e| e.to_string())?;
    std::fs::rename(&fresh, &dir).map_err(|e| e.to_string())?;
    std::fs::remove_dir_all(&old).map_err(|e| e.to_string())?;
    eprintln!("compacted {}", dir.display());
    Ok(())
}
