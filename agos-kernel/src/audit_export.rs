use async_trait::async_trait;
use serde::Serialize;

use crate::security::AuditEntry;

#[async_trait]
pub trait AuditExporter: Send + Sync {
    async fn export(&self, entries: &[AuditEntry]) -> Result<(), String>;
}

pub struct StdoutExporter;

#[async_trait]
impl AuditExporter for StdoutExporter {
    async fn export(&self, entries: &[AuditEntry]) -> Result<(), String> {
        for entry in entries {
            println!(
                "[AUDIT] {} | {} | {} | {}",
                entry.timestamp, entry.result, entry.reason, entry.action
            );
        }
        Ok(())
    }
}

pub struct JsonlExporter {
    path: String,
}

impl JsonlExporter {
    pub fn new(path: &str) -> Self {
        JsonlExporter {
            path: path.to_string(),
        }
    }
}

#[async_trait]
impl AuditExporter for JsonlExporter {
    async fn export(&self, entries: &[AuditEntry]) -> Result<(), String> {
        let mut output = String::new();
        for entry in entries {
            let line = serde_json::to_string(entry)
                .map_err(|e| format!("Serialize audit entry: {}", e))?;
            output.push_str(&line);
            output.push('\n');
        }
        tokio::fs::write(&self.path, output)
            .await
            .map_err(|e| format!("Write audit export: {}", e))?;
        Ok(())
    }
}

#[allow(dead_code)]
pub struct S3Exporter {
    bucket: String,
    prefix: String,
}

impl S3Exporter {
    pub fn new(bucket: &str, prefix: &str) -> Self {
        S3Exporter {
            bucket: bucket.to_string(),
            prefix: prefix.to_string(),
        }
    }
}

#[async_trait]
impl AuditExporter for S3Exporter {
    async fn export(&self, _entries: &[AuditEntry]) -> Result<(), String> {
        Err("S3Exporter requires the `aws-sdk-s3` crate which is not yet linked".to_string())
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum ExportFormat {
    Jsonl,
    Csv,
}

impl ExportFormat {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "jsonl" => Ok(ExportFormat::Jsonl),
            "csv" => Ok(ExportFormat::Csv),
            _ => Err(format!("Unknown export format: {}", s)),
        }
    }
}

#[derive(Serialize)]
pub struct ExportResult {
    pub format: String,
    pub entries_exported: usize,
    pub sink: String,
    pub timestamp: String,
}

pub fn format_entries(entries: &[AuditEntry], format: ExportFormat) -> Result<String, String> {
    match format {
        ExportFormat::Jsonl => {
            let mut out = String::new();
            for entry in entries {
                let line = serde_json::to_string(entry)
                    .map_err(|e| format!("Serialize: {}", e))?;
                out.push_str(&line);
                out.push('\n');
            }
            Ok(out)
        }
        ExportFormat::Csv => {
            let mut out = String::from("timestamp,result,reason,action\n");
            for entry in entries {
                let action_str = serde_json::to_string(&entry.action)
                    .unwrap_or_default();
                out.push_str(&format!(
                    "{},{},{},{}\n",
                    entry.timestamp,
                    entry.result,
                    entry.reason.replace(',', ";"),
                    action_str.replace(',', ";"),
                ));
            }
            Ok(out)
        }
    }
}
