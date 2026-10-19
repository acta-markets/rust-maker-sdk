use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use solana_rpc_client::api::{client_error, request::RpcRequest};
use solana_rpc_client::rpc_client::RpcClientConfig;
use solana_rpc_client::rpc_sender::{RpcSender, RpcTransportStats};

use super::*;

/// Serves account reads and filtered program scans from a fixed account set;
/// any other request panics.
struct Accounts(BTreeMap<Pubkey, Account>);

fn ui(account: &Account) -> Value {
    json!({
        "owner": account.owner.to_string(),
        "lamports": account.lamports,
        "executable": false,
        "rentEpoch": 0,
        "space": account.data.len(),
        "data": [STANDARD.encode(&account.data), "base64"],
    })
}

fn matches(account: &Account, filter: &Value) -> bool {
    if let Some(size) = filter["dataSize"].as_u64() {
        return account.data.len() as u64 == size;
    }
    let memcmp = &filter["memcmp"];
    let offset = memcmp["offset"].as_u64().expect("memcmp offset") as usize;
    let bytes = bs58::decode(memcmp["bytes"].as_str().expect("memcmp bytes"))
        .into_vec()
        .expect("base58 memcmp bytes");
    account.data.get(offset..offset + bytes.len()) == Some(bytes.as_slice())
}

#[async_trait::async_trait]
impl RpcSender for Accounts {
    async fn send(&self, request: RpcRequest, params: Value) -> client_error::Result<Value> {
        let read = |key: &Value| {
            let key: Pubkey = key.as_str().expect("address").parse().expect("pubkey");
            self.0.get(&key).map_or(Value::Null, ui)
        };
        let value = match request {
            RpcRequest::GetAccountInfo => read(&params[0]),
            RpcRequest::GetMultipleAccounts => params[0]
                .as_array()
                .expect("addresses")
                .iter()
                .map(read)
                .collect(),
            RpcRequest::GetProgramAccounts => {
                let program: Pubkey = params[0]
                    .as_str()
                    .expect("program")
                    .parse()
                    .expect("pubkey");
                let filters = params[1]["filters"].as_array().cloned().unwrap_or_default();
                return Ok(self
                    .0
                    .iter()
                    .filter(|(_, account)| {
                        account.owner == program && filters.iter().all(|filter| matches(account, filter))
                    })
                    .map(|(address, account)| json!({"pubkey": address.to_string(), "account": ui(account)}))
                    .collect());
            }
            other => panic!("unexpected RPC {other}"),
        };
        Ok(json!({"context": {"slot": 1}, "value": value}))
    }

    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }

    fn url(&self) -> String {
        "mock://accounts".into()
    }
}

impl ChainClient {
    pub(crate) fn with_accounts(
        program_id: Pubkey,
        accounts: impl IntoIterator<Item = (Pubkey, Account)>,
    ) -> Self {
        let commitment = CommitmentConfig::confirmed();
        Self {
            rpc: RpcClient::new_sender(
                Accounts(accounts.into_iter().collect()),
                RpcClientConfig::with_commitment(commitment),
            ),
            program_id,
            commitment,
        }
    }

    pub(crate) fn lookup_table_account(addresses: &[Pubkey]) -> Account {
        let mut data = vec![0; LOOKUP_TABLE_META_LEN];
        data[..4].copy_from_slice(&1u32.to_le_bytes());
        data[4..12].copy_from_slice(&u64::MAX.to_le_bytes());
        for address in addresses {
            data.extend_from_slice(address.as_ref());
        }
        Account {
            lamports: 1,
            data,
            owner: ADDRESS_LOOKUP_TABLE_PROGRAM_ID,
            executable: false,
            rent_epoch: 0,
        }
    }
}
