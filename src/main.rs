mod aggregator;
mod config;
mod log_watcher;
mod monitor;

use anyhow::{anyhow, Result};
use log::error;
use monitor::Monitor;
use std::process::exit;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("ramon=info"))
        .init();

    if let Err(err) = run().await {
        eprintln!("{err}");
        exit(1);
    }
}

async fn run() -> Result<()> {
    let doc = include_str!("../ramon.toml");
    let config = config::parse(doc).map_err(|err| {
        anyhow!(
            r#"Failed to parse ramon.toml: {err}

Refer to https://github.com/reujab/ramon#specification-wip"#
        )
    })?;

    // Process monitors.
    let mut monitors = Vec::with_capacity(config.monitors.len());
    for monitor_config in config.monitors {
        let name = monitor_config.name.clone();
        let aggregator_id = match &monitor_config.notify {
            None => "default",
            Some(notify) => &notify.r#type,
        };
        let aggregator = config.aggregator_txs.get(aggregator_id).ok_or(anyhow!(
            "Could not find notification config for {aggregator_id:?}"
        ))?;
        let monitor = Monitor::new(monitor_config, aggregator.clone())
            .await
            .map_err(|err| anyhow!("Monitor `{}`: {err}", name))?;
        monitors.push(monitor);
    }
    let mut handles = Vec::with_capacity(monitors.len());
    for mut monitor in monitors {
        let handle = tokio::spawn(async move {
            let res = monitor.start().await;
            if let Err(err) = &res {
                error!("[{}] {err}", monitor.name);
            }
            error!("[{}] Monitor exited early.", monitor.name);
            res
        });
        handles.push(handle);
    }
    for handle in handles {
        handle.await??;
    }

    Ok(())
}
