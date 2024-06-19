use std::collections::BinaryHeap;
use std::sync::{Arc, Mutex};
use alloy::primitives::Address;
use antelope::api::client::{APIClient, DefaultProvider};
use antelope::api::v1::structs::{GetTableRowsParams, IndexPosition, TableIndexType};
use antelope::chain::name::Name;
use moka::sync::Cache;
use crate::block::Block;
use crate::types::evm_types::AccountRow;
use crate::types::names::EOSIO_EVM;

pub struct NameToAddressCache {
    cache: Cache<u64, Address>,
    api_client: APIClient<DefaultProvider>
}

impl NameToAddressCache {
    pub fn new(api_client: APIClient<DefaultProvider>) -> Self {
        NameToAddressCache {
            cache: Cache::new(10_000),
            api_client
        }
    }

    pub async fn get(&self, name: u64) -> Option<Address> {
        let cached = self.cache.get(&name);
        if let Some(cached) = cached {
            Some(cached.clone())
        } else {
            let EVM_CONTRACT = Name::from_u64(EOSIO_EVM);
            // TODO: hardcode this in names.rs for performance
            let ACCOUNT = Name::new_from_str("account");
            let account_result = self.api_client.v1_chain.get_table_rows::<AccountRow>(
                GetTableRowsParams {
                    code: EVM_CONTRACT,
                    table: ACCOUNT,
                    scope: Some(EVM_CONTRACT),
                    lower_bound: Some(TableIndexType::UINT64(name)),
                    upper_bound: Some(TableIndexType::UINT64(name)),
                    limit: Some(1),
                    reverse: None,
                    index_position: Some(IndexPosition::TERTIARY),
                    show_payer: None,
                }
            ).await.unwrap();
            if account_result.rows.len() == 0 {
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
    heap: Arc<Mutex<BinaryHeap<Block>>>,
}

impl PriorityQueue {
    pub fn new() -> Self {
        PriorityQueue {
            heap: Arc::new(Mutex::new(BinaryHeap::new())),
        }
    }

    pub fn push(&self, item: Block) {
        let mut heap = self.heap.lock().unwrap();
        heap.push(item);
    }

    pub fn pop(&self) -> Option<Block> {
        let mut heap = self.heap.lock().unwrap();
        heap.pop()
    }

    pub fn len(&self) -> usize {
        let heap = self.heap.lock().unwrap();
        heap.len()
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
