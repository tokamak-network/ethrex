//! Sentinel — Real-Time Hack Detection System
//!
//! Pre-filters every transaction receipt in a block using lightweight heuristics,
//! flagging suspicious transactions for deep analysis via the Autopsy Lab pipeline.

pub mod alert;
pub mod analyzer;
pub mod history;
pub mod metrics;
pub mod pre_filter;
pub mod replay;
pub mod service;
pub mod types;
#[cfg(feature = "autopsy")]
pub mod webhook;
pub mod ws_broadcaster;

#[cfg(test)]
mod tests;
