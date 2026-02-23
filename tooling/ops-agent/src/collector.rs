use crate::models::TelemetrySnapshot;
use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct Collector {
    client: Client,
    prometheus_base_url: String,
    execution_rpc_url: String,
    block_height_query: String,
    rpc_timeout_rate_query: String,
    cpu_usage_query: String,
}

#[derive(Debug, Error)]
pub enum CollectorError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("prometheus returned non-success status: {0}")]
    PrometheusStatus(String),
    #[error("prometheus response missing value")]
    MissingPrometheusValue,
    #[error("parse float error: {0}")]
    ParseFloat(#[from] std::num::ParseFloatError),
    #[error("rpc response missing result")]
    MissingRpcResult,
    #[error("invalid block height value from prometheus: {0}")]
    InvalidBlockHeight(f64),
    #[error("rpc result parse error: {0}")]
    ParseInt(#[from] std::num::ParseIntError),
}

impl Collector {
    pub fn new(prometheus_base_url: String, execution_rpc_url: String) -> Self {
        Self {
            client: Client::new(),
            prometheus_base_url,
            execution_rpc_url,
            block_height_query: "eth_block_height".to_owned(),
            rpc_timeout_rate_query: "execution_rpc_timeout_rate".to_owned(),
            cpu_usage_query: "node_cpu_usage_percent".to_owned(),
        }
    }

    pub fn with_queries(
        mut self,
        block_height_query: String,
        rpc_timeout_rate_query: String,
        cpu_usage_query: String,
    ) -> Self {
        self.block_height_query = block_height_query;
        self.rpc_timeout_rate_query = rpc_timeout_rate_query;
        self.cpu_usage_query = cpu_usage_query;
        self
    }

    pub async fn collect_snapshot(&self) -> Result<TelemetrySnapshot, CollectorError> {
        let block_height_from_prom = self.query_prometheus_scalar(&self.block_height_query).await?;
        let block_height_from_rpc = self.fetch_rpc_block_number().await?;
        let block_height = self.merge_block_height(block_height_from_prom, block_height_from_rpc)?;

        let execution_rpc_timeout_rate = self
            .query_prometheus_scalar(&self.rpc_timeout_rate_query)
            .await?;
        let cpu_usage_percent = self.query_prometheus_scalar(&self.cpu_usage_query).await?;

        Ok(TelemetrySnapshot {
            captured_at: SystemTime::now(),
            block_height,
            execution_rpc_timeout_rate,
            cpu_usage_percent,
        })
    }

    fn merge_block_height(
        &self,
        block_height_from_prom: f64,
        block_height_from_rpc: u64,
    ) -> Result<u64, CollectorError> {
        if !block_height_from_prom.is_finite() || block_height_from_prom < 0.0 {
            return Err(CollectorError::InvalidBlockHeight(block_height_from_prom));
        }

        let prom_truncated = block_height_from_prom.trunc();
        if prom_truncated > u64::MAX as f64 {
            return Err(CollectorError::InvalidBlockHeight(block_height_from_prom));
        }

        let prom_height = prom_truncated as u64;
        Ok(std::cmp::max(prom_height, block_height_from_rpc))
    }

    async fn query_prometheus_scalar(&self, query: &str) -> Result<f64, CollectorError> {
        let url = format!("{}/api/v1/query", self.prometheus_base_url.trim_end_matches('/'));
        let response = self
            .client
            .get(url)
            .query(&[("query", query)])
            .send()
            .await?
            .json::<PrometheusQueryResponse>()
            .await?;

        if response.status != "success" {
            return Err(CollectorError::PrometheusStatus(response.status));
        }

        let value = response
            .data
            .result
            .first()
            .and_then(|metric| metric.value.get(1))
            .and_then(|raw| raw.as_str())
            .ok_or(CollectorError::MissingPrometheusValue)?;

        value.parse::<f64>().map_err(CollectorError::from)
    }

    async fn fetch_rpc_block_number(&self) -> Result<u64, CollectorError> {
        let response = self
            .client
            .post(&self.execution_rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_blockNumber",
                "params": [],
                "id": 1
            }))
            .send()
            .await?
            .json::<RpcResponse>()
            .await?;

        let hex_result = response.result.ok_or(CollectorError::MissingRpcResult)?;
        let trimmed = hex_result.trim_start_matches("0x");
        u64::from_str_radix(trimmed, 16).map_err(CollectorError::from)
    }
}

#[derive(Debug, Deserialize)]
struct PrometheusQueryResponse {
    status: String,
    data: PrometheusData,
}

#[derive(Debug, Deserialize)]
struct PrometheusData {
    result: Vec<PrometheusResult>,
}

#[derive(Debug, Deserialize)]
struct PrometheusResult {
    value: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
    result: Option<String>,
}
