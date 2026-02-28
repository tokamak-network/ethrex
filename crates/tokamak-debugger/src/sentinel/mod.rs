//! Sentinel — Real-Time Hack Detection System
//!
//! Pre-filters every transaction receipt in a block using lightweight heuristics,
//! flagging suspicious transactions for deep analysis via the Autopsy Lab pipeline.

pub mod analyzer;
pub mod pre_filter;
pub mod replay;
pub mod service;
pub mod types;

#[cfg(test)]
mod tests;
