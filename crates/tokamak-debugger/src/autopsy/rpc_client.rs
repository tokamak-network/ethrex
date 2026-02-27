//! Thin JSON-RPC HTTP client for Ethereum archive nodes.

use ethrex_common::{Address, H256, U256};
use serde_json::{Value, json};

use crate::error::DebuggerError;

/// Minimal Ethereum JSON-RPC client using blocking HTTP.
pub struct EthRpcClient {
    http: reqwest::blocking::Client,
    url: String,
    block_tag: String,
}

/// Subset of block header fields returned by `eth_getBlockByNumber`.
#[derive(Debug, Clone)]
pub struct RpcBlockHeader {
    pub hash: H256,
    pub number: u64,
    pub timestamp: u64,
    pub gas_limit: u64,
    pub base_fee_per_gas: Option<u64>,
    pub coinbase: Address,
}

/// Subset of transaction fields returned by `eth_getTransactionByHash`.
#[derive(Debug, Clone)]
pub struct RpcTransaction {
    pub from: Address,
    pub to: Option<Address>,
    pub value: U256,
    pub input: Vec<u8>,
    pub gas: u64,
    pub gas_price: Option<u64>,
    pub max_fee_per_gas: Option<u64>,
    pub max_priority_fee_per_gas: Option<u64>,
    pub nonce: u64,
    pub block_number: Option<u64>,
}

impl EthRpcClient {
    pub fn new(url: &str, block_number: u64) -> Self {
        Self {
            http: reqwest::blocking::Client::new(),
            url: url.to_string(),
            block_tag: format!("0x{block_number:x}"),
        }
    }

    pub fn block_number(&self) -> u64 {
        u64::from_str_radix(self.block_tag.trim_start_matches("0x"), 16).unwrap_or(0)
    }

    pub fn eth_get_code(&self, addr: Address) -> Result<Vec<u8>, DebuggerError> {
        let result = self.rpc_call(
            "eth_getCode",
            json!([format!("0x{addr:x}"), &self.block_tag]),
        )?;
        let hex_str = result
            .as_str()
            .ok_or_else(|| DebuggerError::Rpc("eth_getCode: expected string".into()))?;
        hex_decode(hex_str)
    }

    pub fn eth_get_balance(&self, addr: Address) -> Result<U256, DebuggerError> {
        let result = self.rpc_call(
            "eth_getBalance",
            json!([format!("0x{addr:x}"), &self.block_tag]),
        )?;
        parse_u256(&result)
    }

    pub fn eth_get_transaction_count(&self, addr: Address) -> Result<u64, DebuggerError> {
        let result = self.rpc_call(
            "eth_getTransactionCount",
            json!([format!("0x{addr:x}"), &self.block_tag]),
        )?;
        parse_u64(&result)
    }

    pub fn eth_get_storage_at(&self, addr: Address, slot: H256) -> Result<U256, DebuggerError> {
        let result = self.rpc_call(
            "eth_getStorageAt",
            json!([
                format!("0x{addr:x}"),
                format!("0x{slot:x}"),
                &self.block_tag
            ]),
        )?;
        parse_u256(&result)
    }

    pub fn eth_get_block_by_number(
        &self,
        block_number: u64,
    ) -> Result<RpcBlockHeader, DebuggerError> {
        let tag = format!("0x{block_number:x}");
        let result = self.rpc_call("eth_getBlockByNumber", json!([tag, false]))?;
        parse_block_header(&result)
    }

    pub fn eth_get_transaction_by_hash(&self, hash: H256) -> Result<RpcTransaction, DebuggerError> {
        let result = self.rpc_call("eth_getTransactionByHash", json!([format!("0x{hash:x}")]))?;
        parse_transaction(&result)
    }

    pub fn eth_chain_id(&self) -> Result<u64, DebuggerError> {
        let result = self.rpc_call("eth_chainId", json!([]))?;
        parse_u64(&result)
    }

    /// Fetch the target block header (at the client's configured block_tag).
    pub fn eth_get_target_block(&self) -> Result<RpcBlockHeader, DebuggerError> {
        self.eth_get_block_by_number(self.block_number())
    }

    fn rpc_call(&self, method: &str, params: Value) -> Result<Value, DebuggerError> {
        let body = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        });

        let response: Value = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .map_err(|e| DebuggerError::Rpc(format!("{method} request failed: {e}")))?
            .json()
            .map_err(|e| DebuggerError::Rpc(format!("{method} response parse failed: {e}")))?;

        if let Some(error) = response.get("error") {
            return Err(DebuggerError::Rpc(format!("{method} RPC error: {}", error)));
        }

        response
            .get("result")
            .cloned()
            .ok_or_else(|| DebuggerError::Rpc(format!("{method}: missing result field")))
    }
}

// --- Parsing helpers ---

fn hex_decode(hex_str: &str) -> Result<Vec<u8>, DebuggerError> {
    let s = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    if s.is_empty() {
        return Ok(Vec::new());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| DebuggerError::Rpc(format!("hex decode error: {e}")))
        })
        .collect()
}

fn parse_u64(val: &Value) -> Result<u64, DebuggerError> {
    let s = val
        .as_str()
        .ok_or_else(|| DebuggerError::Rpc("expected hex string for u64".into()))?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(s, 16).map_err(|e| DebuggerError::Rpc(format!("u64 parse: {e}")))
}

fn parse_u256(val: &Value) -> Result<U256, DebuggerError> {
    let s = val
        .as_str()
        .ok_or_else(|| DebuggerError::Rpc("expected hex string for U256".into()))?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    U256::from_str_radix(s, 16).map_err(|e| DebuggerError::Rpc(format!("U256 parse: {e}")))
}

fn parse_h256(val: &Value) -> Result<H256, DebuggerError> {
    let s = val
        .as_str()
        .ok_or_else(|| DebuggerError::Rpc("expected hex string for H256".into()))?;
    let bytes = hex_decode(s)?;
    if bytes.len() != 32 {
        return Err(DebuggerError::Rpc(format!(
            "H256 expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    Ok(H256::from_slice(&bytes))
}

fn parse_address(val: &Value) -> Result<Address, DebuggerError> {
    let s = val
        .as_str()
        .ok_or_else(|| DebuggerError::Rpc("expected hex string for Address".into()))?;
    let bytes = hex_decode(s)?;
    if bytes.len() != 20 {
        return Err(DebuggerError::Rpc(format!(
            "Address expected 20 bytes, got {}",
            bytes.len()
        )));
    }
    Ok(Address::from_slice(&bytes))
}

fn parse_block_header(val: &Value) -> Result<RpcBlockHeader, DebuggerError> {
    if val.is_null() {
        return Err(DebuggerError::Rpc("block not found".into()));
    }
    Ok(RpcBlockHeader {
        hash: parse_h256(
            val.get("hash")
                .ok_or_else(|| DebuggerError::Rpc("block missing hash".into()))?,
        )?,
        number: parse_u64(
            val.get("number")
                .ok_or_else(|| DebuggerError::Rpc("block missing number".into()))?,
        )?,
        timestamp: parse_u64(
            val.get("timestamp")
                .ok_or_else(|| DebuggerError::Rpc("block missing timestamp".into()))?,
        )?,
        gas_limit: parse_u64(
            val.get("gasLimit")
                .ok_or_else(|| DebuggerError::Rpc("block missing gasLimit".into()))?,
        )?,
        base_fee_per_gas: val.get("baseFeePerGas").and_then(|v| parse_u64(v).ok()),
        coinbase: parse_address(
            val.get("miner")
                .ok_or_else(|| DebuggerError::Rpc("block missing miner".into()))?,
        )?,
    })
}

fn parse_transaction(val: &Value) -> Result<RpcTransaction, DebuggerError> {
    if val.is_null() {
        return Err(DebuggerError::Rpc("transaction not found".into()));
    }
    Ok(RpcTransaction {
        from: parse_address(
            val.get("from")
                .ok_or_else(|| DebuggerError::Rpc("tx missing from".into()))?,
        )?,
        to: val
            .get("to")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .and_then(|v| parse_address(v).ok()),
        value: parse_u256(
            val.get("value")
                .ok_or_else(|| DebuggerError::Rpc("tx missing value".into()))?,
        )?,
        input: {
            let input_val = val
                .get("input")
                .ok_or_else(|| DebuggerError::Rpc("tx missing input".into()))?;
            hex_decode(input_val.as_str().unwrap_or("0x"))?
        },
        gas: parse_u64(
            val.get("gas")
                .ok_or_else(|| DebuggerError::Rpc("tx missing gas".into()))?,
        )?,
        gas_price: val.get("gasPrice").and_then(|v| parse_u64(v).ok()),
        max_fee_per_gas: val.get("maxFeePerGas").and_then(|v| parse_u64(v).ok()),
        max_priority_fee_per_gas: val
            .get("maxPriorityFeePerGas")
            .and_then(|v| parse_u64(v).ok()),
        nonce: parse_u64(
            val.get("nonce")
                .ok_or_else(|| DebuggerError::Rpc("tx missing nonce".into()))?,
        )?,
        block_number: val.get("blockNumber").and_then(|v| parse_u64(v).ok()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_decode_empty() {
        assert_eq!(hex_decode("0x").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn test_hex_decode_bytes() {
        assert_eq!(
            hex_decode("0xdeadbeef").unwrap(),
            vec![0xde, 0xad, 0xbe, 0xef]
        );
    }

    #[test]
    fn test_parse_u64_hex() {
        let val = json!("0x1a");
        assert_eq!(parse_u64(&val).unwrap(), 26);
    }

    #[test]
    fn test_parse_u256_hex() {
        let val = json!("0xff");
        assert_eq!(parse_u256(&val).unwrap(), U256::from(255));
    }

    #[test]
    fn test_parse_h256() {
        let hex = "0x000000000000000000000000000000000000000000000000000000000000002a";
        let val = json!(hex);
        let h = parse_h256(&val).unwrap();
        assert_eq!(h[31], 0x2a);
    }

    #[test]
    fn test_parse_address() {
        let val = json!("0x0000000000000000000000000000000000000042");
        let addr = parse_address(&val).unwrap();
        assert_eq!(addr, Address::from_low_u64_be(0x42));
    }

    #[test]
    fn test_parse_block_header() {
        let block = json!({
            "hash": "0x000000000000000000000000000000000000000000000000000000000000abcd",
            "number": "0xa",
            "timestamp": "0x5f5e100",
            "gasLimit": "0x1c9c380",
            "baseFeePerGas": "0x3b9aca00",
            "miner": "0x0000000000000000000000000000000000000001"
        });
        let header = parse_block_header(&block).unwrap();
        assert_eq!(header.number, 10);
        assert_eq!(header.timestamp, 100_000_000);
        assert_eq!(header.gas_limit, 30_000_000);
        assert_eq!(header.base_fee_per_gas, Some(1_000_000_000));
    }

    #[test]
    fn test_parse_transaction() {
        let tx = json!({
            "from": "0x0000000000000000000000000000000000000100",
            "to": "0x0000000000000000000000000000000000000042",
            "value": "0x0",
            "input": "0xdeadbeef",
            "gas": "0x5208",
            "gasPrice": "0x3b9aca00",
            "nonce": "0x5",
            "blockNumber": "0xa"
        });
        let parsed = parse_transaction(&tx).unwrap();
        assert_eq!(parsed.from, Address::from_low_u64_be(0x100));
        assert_eq!(parsed.to, Some(Address::from_low_u64_be(0x42)));
        assert_eq!(parsed.gas, 21000);
        assert_eq!(parsed.nonce, 5);
        assert_eq!(parsed.input, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn test_parse_transaction_null_to() {
        let tx = json!({
            "from": "0x0000000000000000000000000000000000000100",
            "to": null,
            "value": "0x0",
            "input": "0x",
            "gas": "0x5208",
            "nonce": "0x0"
        });
        let parsed = parse_transaction(&tx).unwrap();
        assert!(parsed.to.is_none());
    }

    #[test]
    fn test_block_not_found() {
        let result = parse_block_header(&json!(null));
        assert!(result.is_err());
    }

    #[test]
    fn test_tx_not_found() {
        let result = parse_transaction(&json!(null));
        assert!(result.is_err());
    }
}
