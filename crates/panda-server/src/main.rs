//! Thin entrypoint: load YAML config, run async proxy (Phase 1.1 workspace boundary).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn init_observability() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .try_init();
}

fn main() -> anyhow::Result<()> {
    init_observability();

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("panda.yaml"));

    let config = Arc::new(
        panda_config::PandaConfig::load_from_path(&config_path)
            .with_context(|| format!("failed to load config from {}", config_path.display()))?,
    );

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(panda_proxy::run(config))
}
