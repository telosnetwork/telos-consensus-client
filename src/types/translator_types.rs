use crate::block::ProcessingEVMBlock;
use crate::types::evm_types::AccountRow;
use crate::types::names::EOSIO_EVM;
use alloy::primitives::Address;
use antelope::api::client::{APIClient, DefaultProvider};
use antelope::api::v1::structs::{GetTableRowsParams, IndexPosition, TableIndexType};
use antelope::chain::name::Name;
use futures_util::stream::{SplitSink, SplitStream};
use moka::sync::Cache;
use std::collections::BinaryHeap;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::info;

pub type WebsocketTransmitter = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
pub type WebsocketReceiver = SplitStream<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>;

pub struct NameToAddressCache {
    cache: Cache<u64, Address>,
    api_client: APIClient<DefaultProvider>,
}

pub enum BlockOrSkip {
    Block(ProcessingEVMBlock),
    Skip(u64),
}

pub struct RawMessage {
    pub sequence: u64,
    pub bytes: Vec<u8>,
}

impl RawMessage {
    pub fn new(sequence: u64, bytes: Vec<u8>) -> Self {
        RawMessage { sequence, bytes }
    }
}

impl NameToAddressCache {
    pub fn new(api_client: APIClient<DefaultProvider>) -> Self {
        NameToAddressCache {
            cache: Cache::new(10_000),
            api_client,
        }
    }

    pub async fn get(&self, name: u64) -> Option<Address> {
        let cached = self.cache.get(&name);
        info!(
            "getting {} cache hit = {:?}",
            Name::from_u64(name).as_string(),
            cached.is_some()
        );
        if let Some(cached) = cached {
            Some(cached)
        } else {
            let evm_contract = Name::from_u64(EOSIO_EVM);
            // TODO: hardcode this in names.rs for performance
            let account = Name::new_from_str("account");
            let account_result = self
                .api_client
                .v1_chain
                .get_table_rows::<AccountRow>(GetTableRowsParams {
                    code: evm_contract,
                    table: account,
                    scope: Some(evm_contract),
                    lower_bound: Some(TableIndexType::UINT64(name)),
                    upper_bound: Some(TableIndexType::UINT64(name)),
                    limit: Some(1),
                    reverse: None,
                    index_position: Some(IndexPosition::TERTIARY),
                    show_payer: None,
                })
                .await
                .unwrap();
            if account_result.rows.is_empty() {
                return None;
            }

            let address_checksum = account_result.rows[0].address;
            let address = Address::from(address_checksum.data);
            self.cache.insert(name, address);
            Some(address)
        }
    }
}

pub struct PriorityQueue {
    heap: Arc<Mutex<BinaryHeap<ProcessingEVMBlock>>>,
}

impl Default for PriorityQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl PriorityQueue {
    pub fn new() -> Self {
        PriorityQueue {
            heap: Arc::new(Mutex::new(BinaryHeap::new())),
        }
    }

    pub fn push(&self, item: ProcessingEVMBlock) {
        let mut heap = self.heap.lock().unwrap();
        heap.push(item);
    }

    pub fn pop(&self) -> Option<ProcessingEVMBlock> {
        let mut heap = self.heap.lock().unwrap();
        heap.pop()
    }

    pub fn len(&self) -> usize {
        let heap = self.heap.lock().unwrap();
        heap.len()
    }

    pub fn is_empty(&self) -> bool {
        let heap = self.heap.lock().unwrap();
        heap.is_empty()
    }

    pub fn capacity(&self) -> usize {
        let heap = self.heap.lock().unwrap();
        heap.capacity()
    }
}

impl Clone for PriorityQueue {
    fn clone(&self) -> Self {
        PriorityQueue {
            heap: Arc::clone(&self.heap),
        }
    }
}
