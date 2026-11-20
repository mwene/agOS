use std::sync::Arc;
use std::time::Duration;

use agos_kernel::api::create_router;
use agos_kernel::kernel::agOSKernel;
use agos_kernel::umi::UnifiedModelInterface;

#[tokio::main]
async fn main() {
    let use_otel = std::env::var("AGOS_OTLP_ENDPOINT").is_ok();
    if use_otel {
        if let Err(e) = agos_kernel::telemetry::init_tracer() {
            eprintln!("Warning: Failed to init OpenTelemetry tracer: {}", e);
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
                )
                .init();
        }
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .init();
    }

    tracing::info!("Bootstrapping agOS Kernel v0.1.0...");

    let umi = UnifiedModelInterface::new();
    let addr = "127.0.0.1:7400";
    let db_path = std::env::var("AGOS_DB_PATH").ok();
    let kernel = Arc::new(agOSKernel::new(umi, addr, db_path));

    tracing::info!(
        "UMI drivers ready: {}",
        kernel
            .umi
            .list_drivers()
            .iter()
            .map(|d| format!("{} (cap {}, {})", d.name, d.capability_level, d.driver_type))
            .collect::<Vec<_>>()
            .join(", ")
    );

    {
        let kernel_clone = kernel.clone();
        let cfg = kernel_clone.memory.episodic_config();
        let interval_secs = cfg.compression_interval_secs;
        let entry_ttl_secs = cfg.entry_ttl_secs;
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let count = kernel_clone.memory.compress_episodic();
                if count > 0 {
                    tracing::debug!("Episodic compression: pruned {} entries", count);
                }
            }
        });
        tracing::info!(
            "Episodic compression daemon started (interval: {}s, TTL: {}s)",
            interval_secs,
            entry_ttl_secs,
        );
    }

    let app = create_router(kernel);

    let addr = "127.0.0.1:7400";
    tracing::info!("agOS Kernel HTTP API listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();

    agos_kernel::telemetry::shutdown_tracer();
}
