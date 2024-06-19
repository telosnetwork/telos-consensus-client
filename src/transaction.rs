use alloy::hex;
use alloy::primitives::{Address, B256, Signature, U256};
use alloy::primitives::TxKind::Call;
use alloy_consensus::{SignableTransaction, Signed, TxLegacy};
use antelope::api::client::Provider;
use antelope::chain::checksum::Checksum256;
use num_bigint::{BigUint, ToBigUint};
use crate::types::evm_types::{RawAction, PrintedReceipt, TransferAction, WithdrawAction};
use crate::types::types::NameToAddressCache;

fn make_unique_vrs(block_hash_native: Checksum256, sender_address: Address, trx_index: usize) -> Signature {
    let v = 42u64;
    let hash_biguint = BigUint::from_bytes_be(&block_hash_native.data);
    let trx_index_biguint: BigUint = trx_index.to_biguint().unwrap();
    let r_biguint = hash_biguint + trx_index_biguint;

    let r = U256::from_be_slice(r_biguint.to_bytes_be().as_slice());
    let s = U256::from_be_slice(sender_address.as_slice());
    Signature::from_rs_and_parity(r, s, v).expect("Failed to create signature")
}

#[derive(Clone)]
pub enum Transaction {
    LegacySigned(Signed<TxLegacy>)
}

impl Transaction {
    pub async fn from_raw_action(raw: RawAction, receipt: PrintedReceipt, native_to_evm_cache: &NameToAddressCache) -> Self {
        // TODO: Check for unsigned transactions and handle correctly
        // TODO: Handle 1559
        let signed_legacy = TxLegacy::decode_signed_fields(&mut raw.tx.clone().as_slice()).unwrap();
        Transaction::LegacySigned(signed_legacy)
    }

    pub async fn from_transfer(chain_id: u64, trx_index: usize, block_hash: Checksum256, action: TransferAction, native_to_evm_cache: &NameToAddressCache) -> Self {
        let mut address: Address;
        if action.memo.starts_with("0x") {
            address = action.memo.parse().unwrap();
        } else {
            address = native_to_evm_cache.get(action.from.n).await.expect("Failed to get address");
        }

        let value = U256::from(action.quantity.amount()) * U256::from(100_000_000_000_000i64);

        let mut tx_legacy = TxLegacy {
            chain_id: Some(chain_id),
            nonce: 0,
            gas_price: 0,
            gas_limit: 21_000,
            to: Call(address),
            value,
            input: Default::default(),
        };

        let sig = make_unique_vrs(block_hash, Address::ZERO, trx_index);
        let signed_legacy = tx_legacy.into_signed(sig);
        Transaction::LegacySigned(signed_legacy)
    }

    pub async fn from_withdraw(chain_id: u64, trx_index: usize, block_hash: Checksum256, action: WithdrawAction, native_to_evm_cache: &NameToAddressCache) -> Self {
        let address = native_to_evm_cache.get(action.to.n).await.expect("Failed to get address");
        let value = U256::from(action.quantity.amount()) * U256::from(100_000_000_000_000i64);
        let mut tx_legacy = TxLegacy {
            chain_id: Some(chain_id),
            nonce: 0,
            gas_price: 0,
            gas_limit: 21_000,
            to: Call(Address::ZERO),
            value,
            input: Default::default(),
        };

        let sig = make_unique_vrs(block_hash, address, trx_index);
        let signed_legacy = tx_legacy.into_signed(sig);
        Transaction::LegacySigned(signed_legacy)
    }

    pub fn hash(&self) -> &B256 {
        match self {
            Transaction::LegacySigned(signed_legacy) => {
                signed_legacy.hash()
            }
        }
    }
}