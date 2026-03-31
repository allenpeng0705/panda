//! Thin entrypoint: load YAML config, run async proxy (Phase 1.1 workspace boundary).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use opentelemetry::trace::TracerProvider;
use opentelemetry::KeyValue;
use opentelemetry::global;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// When OTLP is enabled, returns a clone of the SDK provider for shutdown after the runtime stops
/// (OpenTelemetry 0.31 removed `global::shutdown_tracer_provider`).
fn init_observability() -> Option<SdkTracerProvider> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer().json();

    if let Some(endpoint) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        let service_name = std::env::var("PANDA_OTEL_SERVICE_NAME")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "panda-gateway".to_string());
        match build_otel_provider(&endpoint, &service_name) {
            Ok(provider) => {
                let shutdown_handle = provider.clone();
                let tracer = provider.tracer("panda");
                global::set_tracer_provider(provider);
                let _ = tracing_subscriber::registry()
                    .with(filter)
                    .with(fmt_layer)
                    .with(tracing_opentelemetry::layer().with_tracer(tracer))
                    .try_init();
                return Some(shutdown_handle);
            }
            Err(e) => {
                eprintln!("panda: failed to init OTLP exporter ({e}); falling back to logs-only tracing");
                let _ = tracing_subscriber::registry()
                    .with(filter)
                    .with(fmt_layer)
                    .try_init();
            }
        }
    } else {
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .try_init();
    }
    None
}

fn build_otel_provider(
    endpoint: &str,
    service_name: &str,
) -> anyhow::Result<SdkTracerProvider> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .build()?;
    Ok(SdkTracerProvider::builder()
        .with_resource(
            Resource::builder_empty()
                .with_attributes([KeyValue::new("service.name", service_name.to_string())])
                .build(),
        )
        .with_batch_exporter(exporter)
        .build())
}

fn main() -> anyhow::Result<()> {
    let otel_provider = init_observability();

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

    let out = rt.block_on(panda_proxy::run(config));
    if let Some(p) = otel_provider {
        let _ = p.shutdown();
    }
    out
}
