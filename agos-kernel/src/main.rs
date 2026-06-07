use std::sync::Arc;

use agos_kernel::api::create_router;
use agos_kernel::kernel::agOSKernel;
use agos_kernel::umi::UnifiedModelInterface;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Bootstrapping agOS Kernel v0.1.0...");

    let umi = UnifiedModelInterface::new();
    let kernel = Arc::new(agOSKernel::new(umi));

    tracing::info!(
        "UMI drivers ready: LocalEngine-v0 (cap 2), CloudGPT-HighReason (cap 5)"
    );

    let app = create_router(kernel);

    let addr = "127.0.0.1:7400";
    tracing::info!("agOS Kernel HTTP API listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
