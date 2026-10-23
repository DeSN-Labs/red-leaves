use crate::identity::Error;
use alloy_sol_types::{SolCall, sol};
use red_leaves_protocol::Address;
use serde_json::{Value, json};

sol! {
    struct RegistryUser { string nasPeerId; bytes encryptionPublicKey; string nickname; }
    function getUser(address owner) external view returns (RegistryUser);
}
pub struct Chain {
    client: reqwest::Client,
    rpc: String,
    pub registry: Address,
}
pub struct Profile {
    pub peer: String,
    pub public_key: Vec<u8>,
}
impl Chain {
    pub fn new(rpc: String, registry: Address) -> Result<Self, Error> {
        let url = reqwest::Url::parse(&rpc)?;
        if url.scheme() != "https"
            && !(url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))
        {
            return Err("RPC requires HTTPS outside loopback".into());
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(8))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            rpc,
            registry,
        })
    }
    async fn rpc(&self, method: &str, params: Value) -> Result<Value, Error> {
        let start = std::time::Instant::now();
        let result: Result<Value, Error> = async {
            let mut response = self
                .client
                .post(&self.rpc)
                .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
                .send()
                .await?
                .error_for_status()?;
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if bytes.len() + chunk.len() > 64 * 1024 {
                    return Err("RPC response too large".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            let value: Value = serde_json::from_slice(&bytes)?;
            if value.get("error").is_some() {
                return Err("RPC request failed".into());
            }
            value
                .get("result")
                .cloned()
                .ok_or_else(|| "Invalid RPC response".into())
        }
        .await;
        if result.is_err() {
            tracing::warn!(
                rpc_method = method,
                elapsed_ms = start.elapsed().as_millis() as u64,
                "Registry RPC request failed"
            );
        } else {
            tracing::debug!(
                rpc_method = method,
                elapsed_ms = start.elapsed().as_millis() as u64,
                "Registry RPC request completed"
            );
        }
        result
    }
    async fn block(&self, confirmations: u64) -> Result<String, Error> {
        if self.rpc("eth_chainId", json!([])).await? != "0x14a34" {
            return Err("Wrong RPC chain".into());
        }
        let value = self.rpc("eth_blockNumber", json!([])).await?;
        let block = u64::from_str_radix(
            value
                .as_str()
                .ok_or("Invalid block")?
                .trim_start_matches("0x"),
            16,
        )?;
        Ok(format!("0x{:x}", block.saturating_sub(confirmations)))
    }
    async fn call<C: SolCall>(
        &self,
        contract: Address,
        call: C,
        block: &str,
    ) -> Result<Vec<u8>, Error> {
        let result = self.rpc("eth_call",json!([{"to":format!("0x{}",hex::encode(contract.0)),"data":format!("0x{}",hex::encode(call.abi_encode()))},block])).await?;
        Ok(hex::decode(
            result
                .as_str()
                .ok_or("Invalid contract response")?
                .strip_prefix("0x")
                .ok_or("Invalid contract bytes")?,
        )?)
    }
    pub async fn profile(&self, owner: Address) -> Result<Profile, Error> {
        let block = self.block(0).await?;
        let bytes = self
            .call(
                self.registry,
                getUserCall {
                    owner: owner.0.into(),
                },
                &block,
            )
            .await?;
        let profile = getUserCall::abi_decode_returns(&bytes)?;
        if profile.encryptionPublicKey.len() != 32
            || profile.nasPeerId.parse::<libp2p::PeerId>().is_err()
            || profile.nasPeerId.len() > 128
            || profile.nasPeerId.is_empty()
        {
            return Err("Unsupported profile encryption key".into());
        }
        Ok(Profile {
            peer: profile.nasPeerId,
            public_key: profile.encryptionPublicKey.to_vec(),
        })
    }
}
