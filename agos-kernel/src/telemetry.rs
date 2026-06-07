use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::runtime::Tokio;
use opentelemetry_sdk::trace::Config;
use opentelemetry_sdk::Resource;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

pub fn init_tracer() -> Result<(), String> {
    let endpoint = std::env::var("AGOS_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4317".to_string());
    let ep = endpoint.clone();

    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(endpoint),
        )
        .with_trace_config(
            Config::default().with_resource(Resource::new(vec![
                KeyValue::new("service.name", "agos-kernel"),
                KeyValue::new("service.version", "0.1.0"),
            ])),
        )
        .install_batch(Tokio)
        .map_err(|e| format!("Failed to install OTLP tracer: {}", e))?;

    let telemetry_layer = tracing_opentelemetry::layer()
        .with_tracer(tracer);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(telemetry_layer)
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("OpenTelemetry tracer initialized, exporting to {}", &ep);
    Ok(())
}

pub fn shutdown_tracer() {
    opentelemetry::global::shutdown_tracer_provider();
}
