//! Unified L2 State Layer
//!
//! Single source of truth for all L2 instance state (appchains + Docker deployments).
//! Background-refreshed every 5 seconds, consumed by Telegram Bot, AI Messenger, and Frontend.

use crate::appchain_manager::{AppchainManager, AppchainStatus};
use crate::deployment_db::{self, DeploymentProxy, MonitoringInfo, RpcHealth};
use crate::pilot_memory::PilotMemory;
use crate::runner::ProcessRunner;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::sync::broadcast;

const LOCAL_SERVER_URL: &str = "http://127.0.0.1:5002";
const REFRESH_INTERVAL_SECS: u64 = 5;
const EVENT_CHANNEL_CAPACITY: usize = 64;

// ── Data Models ──

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum L2Source {
    Appchain,
    Deployment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum L2Status {
    Created,
    SettingUp,
    Running,
    Stopped,
    Error,
    Partial,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2Health {
    pub l1_healthy: bool,
    pub l2_healthy: bool,
    pub l1_block_number: Option<serde_json::Value>,
    pub l2_block_number: Option<serde_json::Value>,
    pub l1_chain_id: Option<serde_json::Value>,
    pub l2_chain_id: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2Contracts {
    pub bridge: Option<String>,
    pub proposer: Option<String>,
    pub timelock: Option<String>,
    pub sp1_verifier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2Container {
    pub service: String,
    pub state: String,
    pub status: String,
}

/// Unified L2 instance info combining appchain config + deployment runtime state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2Info {
    pub id: String,
    pub name: String,
    pub source: L2Source,

    // Config
    pub chain_id: Option<u64>,
    pub network_mode: String,
    pub native_token: String,

    // Runtime state
    pub status: L2Status,
    pub health: Option<L2Health>,

    // Endpoints
    pub l1_rpc_url: Option<String>,
    pub l2_rpc_url: Option<String>,

    // Contracts
    pub contracts: Option<L2Contracts>,

    // Docker (Deployment only)
    pub containers: Option<Vec<L2Container>>,
    pub phase: Option<String>,
    pub error_message: Option<String>,

    // Meta
    pub is_public: bool,
    pub created_at: String,
}

// ── Events ──

#[derive(Debug, Clone, Serialize)]
pub struct L2Event {
    pub event_type: String,
    pub l2_id: String,
    pub l2_name: String,
    pub source_type: String, // "appchain" | "deployment"
    pub detail: String,
    pub timestamp: String,
}

// ── Snapshot ──

#[derive(Debug, Clone)]
struct StateSnapshot {
    items: Vec<L2Info>,
    refreshed_at: Instant,
}

// ── UnifiedL2State ──

pub struct UnifiedL2State {
    snapshot: RwLock<StateSnapshot>,
    event_tx: broadcast::Sender<L2Event>,
}

impl UnifiedL2State {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            snapshot: RwLock::new(StateSnapshot {
                items: Vec::new(),
                refreshed_at: Instant::now(),
            }),
            event_tx,
        }
    }

    /// Get all L2 instances (cached, instant)
    pub fn get_all(&self) -> Vec<L2Info> {
        self.snapshot.read().unwrap().items.clone()
    }

    /// Get a specific L2 instance by ID
    pub fn get_by_id(&self, id: &str) -> Option<L2Info> {
        self.snapshot
            .read()
            .unwrap()
            .items
            .iter()
            .find(|l| l.id == id)
            .cloned()
    }

    /// Build JSON context for AI system prompts (Telegram + Desktop Chat)
    pub fn to_context_json(&self) -> serde_json::Value {
        let snap = self.snapshot.read().unwrap();
        let appchains: Vec<&L2Info> = snap
            .items
            .iter()
            .filter(|l| l.source == L2Source::Appchain)
            .collect();
        let deployments: Vec<&L2Info> = snap
            .items
            .iter()
            .filter(|l| l.source == L2Source::Deployment)
            .collect();

        serde_json::json!({
            "appchains": appchains,
            "deployments": deployments,
            "total_appchains": appchains.len(),
            "total_deployments": deployments.len(),
        })
    }

    /// Subscribe to state change events
    pub fn subscribe_events(&self) -> broadcast::Receiver<L2Event> {
        self.event_tx.subscribe()
    }

    /// Emit an event
    fn emit_event(&self, event: L2Event) {
        // Ignore send errors (no receivers)
        let _ = self.event_tx.send(event);
    }

    /// Force immediate refresh (after an action)
    pub async fn refresh_now(
        &self,
        am: &AppchainManager,
        runner: &ProcessRunner,
    ) {
        self.do_refresh(am, runner).await;
    }

    /// Core refresh logic: collect state from AppchainManager + local-server
    async fn do_refresh(
        &self,
        am: &AppchainManager,
        runner: &ProcessRunner,
    ) {
        let old_items = self.snapshot.read().unwrap().items.clone();
        let mut new_items = Vec::new();

        // 1. Appchains from AppchainManager
        let chains = am.list_appchains();
        for chain in &chains {
            let process_alive = runner.is_running(&chain.id).await;
            let status = reconcile_appchain_status(&chain.status, process_alive);

            // If status diverged, update AppchainManager
            if status != appchain_status_to_l2(&chain.status) {
                match status {
                    L2Status::Error => am.update_status(&chain.id, AppchainStatus::Error),
                    L2Status::Stopped => am.update_status(&chain.id, AppchainStatus::Stopped),
                    _ => {}
                }
            }

            new_items.push(L2Info {
                id: chain.id.clone(),
                name: chain.name.clone(),
                source: L2Source::Appchain,
                chain_id: Some(chain.chain_id),
                network_mode: format!("{:?}", chain.network_mode),
                native_token: chain.native_token.clone(),
                status,
                health: None, // Appchains don't have monitoring endpoint yet
                l1_rpc_url: Some(chain.l1_rpc_url.clone()),
                l2_rpc_url: Some(format!("http://localhost:{}", chain.l2_rpc_port)),
                contracts: if chain.bridge_address.is_some() || chain.on_chain_proposer_address.is_some() {
                    Some(L2Contracts {
                        bridge: chain.bridge_address.clone(),
                        proposer: chain.on_chain_proposer_address.clone(),
                        timelock: None,
                        sp1_verifier: None,
                    })
                } else {
                    None
                },
                containers: None,
                phase: None,
                error_message: None,
                is_public: chain.is_public,
                created_at: chain.created_at.clone(),
            });
        }

        // 2. Docker deployments from local-server
        let deployments = deployment_db::list_deployments_from_db().unwrap_or_default();
        let proxy = DeploymentProxy::new(LOCAL_SERVER_URL);

        for dep in &deployments {
            // Fetch container status
            let containers = proxy.get_containers(&dep.id).await.unwrap_or_default();
            let container_info: Vec<L2Container> = containers
                .iter()
                .map(|c| L2Container {
                    service: c.service.clone(),
                    state: c.state.clone(),
                    status: c.status.clone(),
                })
                .collect();

            // Determine live status from containers
            let status = if containers.is_empty() {
                match dep.phase.as_str() {
                    "running" | "active" => L2Status::Stopped,
                    "error" => L2Status::Error,
                    "stopped" => L2Status::Stopped,
                    "configured" => L2Status::Created,
                    _ => {
                        // Provisioning phases
                        if ["checking_docker", "building", "pulling", "l1_starting",
                            "deploying_contracts", "verifying_contracts", "l2_starting",
                            "starting_prover", "starting_tools"]
                            .contains(&dep.phase.as_str())
                        {
                            L2Status::SettingUp
                        } else {
                            L2Status::Stopped
                        }
                    }
                }
            } else if containers.iter().all(|c| c.state == "running") {
                L2Status::Running
            } else if containers.iter().all(|c| c.state == "exited" || c.state == "dead") {
                L2Status::Stopped
            } else {
                L2Status::Partial
            };

            // Fetch monitoring (best-effort)
            let monitoring = proxy.get_monitoring(&dep.id).await.ok();
            let health = monitoring.as_ref().map(|m| build_health(m));

            // Contracts
            let contracts = if dep.bridge_address.is_some() || dep.proposer_address.is_some() {
                Some(L2Contracts {
                    bridge: dep.bridge_address.clone(),
                    proposer: dep.proposer_address.clone(),
                    timelock: dep.timelock_address.clone(),
                    sp1_verifier: dep.sp1_verifier_address.clone(),
                })
            } else {
                None
            };

            let chain_id = dep.chain_id.map(|c| c as u64);

            new_items.push(L2Info {
                id: dep.id.clone(),
                name: dep.name.clone(),
                source: L2Source::Deployment,
                chain_id,
                network_mode: dep
                    .config
                    .as_ref()
                    .and_then(|c| {
                        serde_json::from_str::<serde_json::Value>(c)
                            .ok()
                            .and_then(|v| v["mode"].as_str().map(String::from))
                    })
                    .unwrap_or_else(|| "local".to_string()),
                native_token: "TON".to_string(),
                status,
                health,
                l1_rpc_url: dep.rpc_url.clone().or_else(|| {
                    dep.l1_port.map(|p| format!("http://127.0.0.1:{}", p))
                }),
                l2_rpc_url: dep.l2_port.map(|p| format!("http://127.0.0.1:{}", p)),
                contracts,
                containers: if container_info.is_empty() {
                    None
                } else {
                    Some(container_info)
                },
                phase: Some(dep.phase.clone()),
                error_message: dep.error_message.clone(),
                is_public: dep.is_public != 0,
                created_at: dep.created_at.to_string(),
            });
        }

        // 3. Diff and emit events
        diff_and_emit(&old_items, &new_items, &self.event_tx);

        // 4. Update snapshot
        *self.snapshot.write().unwrap() = StateSnapshot {
            items: new_items,
            refreshed_at: Instant::now(),
        };
    }
}

// ── Background refresh task ──

pub fn spawn_state_refresh(
    state: Arc<UnifiedL2State>,
    am: Arc<AppchainManager>,
    runner: Arc<ProcessRunner>,
    memory: Arc<PilotMemory>,
    app_handle: tauri::AppHandle,
) -> tokio::task::JoinHandle<()> {
    // Pipe events to PilotMemory + Tauri frontend events
    let mut event_rx = state.subscribe_events();
    let memory_clone = memory.clone();
    let app_clone = app_handle.clone();
    tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            // Record in PilotMemory
            memory_clone.append_event(
                &event.event_type,
                &event.l2_name,
                &event.l2_id,
                &event.detail,
                "system",
            );
            // Emit to frontend via Tauri event
            use tauri::Emitter;
            let _ = app_clone.emit("l2-state-changed", &event);
        }
    });

    // Periodic refresh loop
    tokio::spawn(async move {
        loop {
            state.do_refresh(&am, &runner).await;
            tokio::time::sleep(tokio::time::Duration::from_secs(REFRESH_INTERVAL_SECS)).await;
        }
    })
}

// ── Helpers ──

fn reconcile_appchain_status(status: &AppchainStatus, process_alive: bool) -> L2Status {
    match status {
        AppchainStatus::Running if !process_alive => L2Status::Error,
        AppchainStatus::Running => L2Status::Running,
        AppchainStatus::Stopped => L2Status::Stopped,
        AppchainStatus::SettingUp => L2Status::SettingUp,
        AppchainStatus::Error => L2Status::Error,
        AppchainStatus::Created => L2Status::Created,
    }
}

fn appchain_status_to_l2(status: &AppchainStatus) -> L2Status {
    match status {
        AppchainStatus::Running => L2Status::Running,
        AppchainStatus::Stopped => L2Status::Stopped,
        AppchainStatus::SettingUp => L2Status::SettingUp,
        AppchainStatus::Error => L2Status::Error,
        AppchainStatus::Created => L2Status::Created,
    }
}

fn build_health(m: &MonitoringInfo) -> L2Health {
    let (l1_healthy, l1_block, l1_chain) = match &m.l1 {
        Some(l) => (l.healthy, l.block_number.clone(), l.chain_id.clone()),
        None => (false, None, None),
    };
    let (l2_healthy, l2_block, l2_chain) = match &m.l2 {
        Some(l) => (l.healthy, l.block_number.clone(), l.chain_id.clone()),
        None => (false, None, None),
    };
    L2Health {
        l1_healthy,
        l2_healthy,
        l1_block_number: l1_block,
        l2_block_number: l2_block,
        l1_chain_id: l1_chain,
        l2_chain_id: l2_chain,
    }
}

fn diff_and_emit(
    old: &[L2Info],
    new: &[L2Info],
    tx: &broadcast::Sender<L2Event>,
) {
    let now = chrono::Utc::now().to_rfc3339();

    for new_item in new {
        let old_item = old.iter().find(|o| o.id == new_item.id && o.source == new_item.source);

        match old_item {
            Some(old_item) => {
                // Status change
                if old_item.status != new_item.status {
                    let _ = tx.send(L2Event {
                        event_type: "status_changed".to_string(),
                        l2_id: new_item.id.clone(),
                        l2_name: new_item.name.clone(),
                        source_type: format!("{:?}", new_item.source).to_lowercase(),
                        detail: format!(
                            "{:?} → {:?}",
                            old_item.status, new_item.status
                        ),
                        timestamp: now.clone(),
                    });
                }

                // Health change (deployment only)
                if let (Some(old_h), Some(new_h)) = (&old_item.health, &new_item.health) {
                    if old_h.l1_healthy != new_h.l1_healthy || old_h.l2_healthy != new_h.l2_healthy
                    {
                        let _ = tx.send(L2Event {
                            event_type: "health_changed".to_string(),
                            l2_id: new_item.id.clone(),
                            l2_name: new_item.name.clone(),
                            source_type: format!("{:?}", new_item.source).to_lowercase(),
                            detail: format!(
                                "L1: {} → {}, L2: {} → {}",
                                old_h.l1_healthy, new_h.l1_healthy,
                                old_h.l2_healthy, new_h.l2_healthy
                            ),
                            timestamp: now.clone(),
                        });
                    }
                }
            }
            None => {
                // New instance
                let _ = tx.send(L2Event {
                    event_type: "created".to_string(),
                    l2_id: new_item.id.clone(),
                    l2_name: new_item.name.clone(),
                    source_type: format!("{:?}", new_item.source).to_lowercase(),
                    detail: format!("New {:?} instance", new_item.source),
                    timestamp: now.clone(),
                });
            }
        }
    }

    // Detect deletions
    for old_item in old {
        if !new.iter().any(|n| n.id == old_item.id && n.source == old_item.source) {
            let _ = tx.send(L2Event {
                event_type: "deleted".to_string(),
                l2_id: old_item.id.clone(),
                l2_name: old_item.name.clone(),
                source_type: format!("{:?}", old_item.source).to_lowercase(),
                detail: format!("{:?} instance removed", old_item.source),
                timestamp: now.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_l2_status_serialize() {
        let json = serde_json::to_string(&L2Status::Running).unwrap();
        assert_eq!(json, "\"running\"");
        let json = serde_json::to_string(&L2Status::Partial).unwrap();
        assert_eq!(json, "\"partial\"");
    }

    #[test]
    fn test_l2_source_serialize() {
        let json = serde_json::to_string(&L2Source::Appchain).unwrap();
        assert_eq!(json, "\"appchain\"");
        let json = serde_json::to_string(&L2Source::Deployment).unwrap();
        assert_eq!(json, "\"deployment\"");
    }

    #[test]
    fn test_reconcile_running_but_dead() {
        let status = reconcile_appchain_status(&AppchainStatus::Running, false);
        assert_eq!(status, L2Status::Error);
    }

    #[test]
    fn test_reconcile_running_alive() {
        let status = reconcile_appchain_status(&AppchainStatus::Running, true);
        assert_eq!(status, L2Status::Running);
    }

    #[test]
    fn test_reconcile_stopped() {
        let status = reconcile_appchain_status(&AppchainStatus::Stopped, false);
        assert_eq!(status, L2Status::Stopped);
    }

    #[test]
    fn test_empty_state() {
        let state = UnifiedL2State::new();
        assert!(state.get_all().is_empty());
        assert!(state.get_by_id("nonexistent").is_none());
    }

    #[test]
    fn test_context_json_empty() {
        let state = UnifiedL2State::new();
        let ctx = state.to_context_json();
        assert_eq!(ctx["total_appchains"], 0);
        assert_eq!(ctx["total_deployments"], 0);
    }

    #[test]
    fn test_diff_detects_new_item() {
        let (tx, mut rx) = broadcast::channel(16);
        let old: Vec<L2Info> = vec![];
        let new = vec![L2Info {
            id: "test-1".to_string(),
            name: "Test Chain".to_string(),
            source: L2Source::Appchain,
            chain_id: Some(17001),
            network_mode: "Local".to_string(),
            native_token: "TON".to_string(),
            status: L2Status::Created,
            health: None,
            l1_rpc_url: None,
            l2_rpc_url: None,
            contracts: None,
            containers: None,
            phase: None,
            error_message: None,
            is_public: false,
            created_at: "2026-01-01".to_string(),
        }];

        diff_and_emit(&old, &new, &tx);
        let event = rx.try_recv().unwrap();
        assert_eq!(event.event_type, "created");
        assert_eq!(event.l2_id, "test-1");
    }

    #[test]
    fn test_diff_detects_status_change() {
        let (tx, mut rx) = broadcast::channel(16);
        let item = L2Info {
            id: "test-1".to_string(),
            name: "Test Chain".to_string(),
            source: L2Source::Appchain,
            chain_id: Some(17001),
            network_mode: "Local".to_string(),
            native_token: "TON".to_string(),
            status: L2Status::Running,
            health: None,
            l1_rpc_url: None,
            l2_rpc_url: None,
            contracts: None,
            containers: None,
            phase: None,
            error_message: None,
            is_public: false,
            created_at: "2026-01-01".to_string(),
        };

        let mut stopped = item.clone();
        stopped.status = L2Status::Stopped;

        diff_and_emit(&[item], &[stopped], &tx);
        let event = rx.try_recv().unwrap();
        assert_eq!(event.event_type, "status_changed");
        assert!(event.detail.contains("Running"));
        assert!(event.detail.contains("Stopped"));
    }

    #[test]
    fn test_diff_detects_deletion() {
        let (tx, mut rx) = broadcast::channel(16);
        let item = L2Info {
            id: "test-1".to_string(),
            name: "Test Chain".to_string(),
            source: L2Source::Appchain,
            chain_id: Some(17001),
            network_mode: "Local".to_string(),
            native_token: "TON".to_string(),
            status: L2Status::Running,
            health: None,
            l1_rpc_url: None,
            l2_rpc_url: None,
            contracts: None,
            containers: None,
            phase: None,
            error_message: None,
            is_public: false,
            created_at: "2026-01-01".to_string(),
        };

        diff_and_emit(&[item], &[], &tx);
        let event = rx.try_recv().unwrap();
        assert_eq!(event.event_type, "deleted");
    }

    #[test]
    fn test_diff_no_change() {
        let (tx, mut rx) = broadcast::channel(16);
        let item = L2Info {
            id: "test-1".to_string(),
            name: "Test Chain".to_string(),
            source: L2Source::Appchain,
            chain_id: Some(17001),
            network_mode: "Local".to_string(),
            native_token: "TON".to_string(),
            status: L2Status::Running,
            health: None,
            l1_rpc_url: None,
            l2_rpc_url: None,
            contracts: None,
            containers: None,
            phase: None,
            error_message: None,
            is_public: false,
            created_at: "2026-01-01".to_string(),
        };

        diff_and_emit(&[item.clone()], &[item], &tx);
        assert!(rx.try_recv().is_err());
    }
}
