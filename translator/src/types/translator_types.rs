use crate::block::ProcessingEVMBlock;
use crate::types::evm_types::AccountRow;
use crate::types::names::{ACCOUNT, EOSIO_EVM};
use alloy::primitives::Address;
use antelope::api::client::{APIClient, DefaultProvider};
use antelope::api::v1::structs::{
    GetTableRowsParams, GetTableRowsResponse, IndexPosition, TableIndexType,
};
use antelope::chain::name::Name;
use futures_util::stream::{SplitSink, SplitStream};
use moka::sync::Cache;
use reth_primitives::revm_primitives::bitvec::macros::internal::funty::Fundamental;
use serde::{Deserialize, Serialize};
use std::collections::BinaryHeap;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{error, info, warn};

pub type WebsocketTransmitter = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
pub type WebsocketReceiver = SplitStream<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>;

const MAX_RETRY: u8 = 10;
const BASE_DELAY: Duration = Duration::from_millis(20);

pub struct NameToAddressCache {
    cache: Cache<u64, Address>,
    index_cache: Cache<u64, Address>,
    api_client: APIClient<DefaultProvider>,
}

impl NameToAddressCache {
    pub fn new(api_client: APIClient<DefaultProvider>) -> Self {
        NameToAddressCache {
            cache: Cache::new(10_000),
            index_cache: Cache::new(10_000),
            api_client,
        }
    }

    pub async fn get(&self, name: u64) -> Option<Address> {
        let evm_contract = Name::from_u64(EOSIO_EVM);
        let address = Name::from_u64(name).as_string();
        let cached = self.cache.get(&name);

        info!("getting {address} cache hit = {:?}", cached.is_some());

        if cached.is_some() {
            return cached;
        }

        let mut i = 0u8;
        while i <= MAX_RETRY {
            info!("Fetching address {address} try {i}",);
            let account_result = self
                .get_account_address(name, evm_contract, ACCOUNT, IndexPosition::TERTIARY)
                .await;

            let Some(account_row) = account_result.rows.first() else {
                warn!("Got empty rows for {address}, retry attempt {i}");
                sleep(BASE_DELAY * 2u32.pow(i.as_u32())).await;
                i += 1;
                continue;
            };

            let address = Address::from(account_row.address.data);
            self.cache.insert(name, address);
            self.index_cache.insert(account_row.index, address);
            return Some(address);
        }

        error!("Could not get account after {i} attempts for {address}");
        None
    }

    pub async fn get_index(&self, index: u64) -> Option<Address> {
        let evm_contract = Name::from_u64(EOSIO_EVM);
        let address = Name::from_u64(index).as_string();
        let cached = self.index_cache.get(&index);
        info!("getting index {index} cache hit = {:?}", cached.is_some());

        if cached.is_some() {
            return cached;
        }

        let mut i = 0u8;
        while i <= MAX_RETRY {
            info!("Fetching address {address} try attempt {i}");
            let account_result = self
                .get_account_address(index, evm_contract, ACCOUNT, IndexPosition::PRIMARY)
                .await;

            let Some(account_row) = account_result.rows.first() else {
                warn!("Got empty rows for {address}, retry attempt {i}");
                sleep(BASE_DELAY * 2u32.pow(i.as_u32())).await;
                i += 1;
                continue;
            };

            let address = Address::from(account_row.address.data);
            self.cache.insert(account_row.account.value(), address);
            self.index_cache.insert(index, address);
            return Some(address);
        }
        error!("Could not get account after {i} attempts for {address}");
        None
    }

    pub async fn get_account_address(
        &self,
        index: u64,
        evm_contract: Name,
        account: Name,
        index_position: IndexPosition,
    ) -> GetTableRowsResponse<AccountRow> {
        self.api_client
            .v1_chain
            .get_table_rows::<AccountRow>(GetTableRowsParams {
                code: evm_contract,
                table: account,
                scope: Some(evm_contract),
                lower_bound: Some(TableIndexType::UINT64(index)),
                upper_bound: Some(TableIndexType::UINT64(index)),
                limit: Some(1),
                reverse: None,
                index_position: Some(index_position),
                show_payer: None,
            })
            .await
            .unwrap()
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

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ChainId(pub u64);

impl From<u64> for ChainId {
    fn from(value: u64) -> Self {
        ChainId(value)
    }
}

impl ChainId {
    pub fn block_delta(&self) -> u32 {
        match self.0 {
            40 => 36,
            41 => 57,
            _ => 0,
        }
    }
}
