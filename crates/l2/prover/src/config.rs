use serde::Deserialize;
use url::Url;

use crate::backend::BackendType;

#[derive(Deserialize, Debug)]
pub struct ProverConfig {
    pub backend: BackendType,
    pub proof_coordinators: Vec<Url>,
    pub proving_time_ms: u64,
    pub timed: bool,
    #[cfg(all(feature = "sp1", feature = "gpu"))]
    pub sp1_server: Option<Url>,
    /// Optional path to a TOML file that configures which guest programs to load.
    #[serde(default)]
    pub programs_config_path: Option<String>,
    /// Path to the `tokamak-cli` binary.
    #[cfg(feature = "tokamak")]
    #[serde(default)]
    pub tokamak_cli_path: Option<std::path::PathBuf>,
    /// Path to the Tokamak-zk-EVM repository root directory.
    /// The CLI reads/writes to `{resource_dir}/dist/resource/`.
    #[cfg(feature = "tokamak")]
    #[serde(default)]
    pub tokamak_resource_dir: Option<std::path::PathBuf>,
    /// L2 RPC URL for the Tokamak synthesizer to fetch state from.
    /// Required when running the full prove pipeline.
    #[cfg(feature = "tokamak")]
    #[serde(default)]
    pub tokamak_l2_rpc_url: Option<String>,
}
