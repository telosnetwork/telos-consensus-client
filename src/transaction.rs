use alloy::primitives::{Address, B256, Signature, U256};
use alloy::primitives::TxKind::Call;
use alloy_consensus::{SignableTransaction, Signed, TxLegacy};
use antelope::api::client::Provider;
use antelope::chain::checksum::Checksum256;
use antelope::util::bytes_to_hex;
use num_bigint::{BigUint, ToBigUint};
use tracing::info;
use crate::rlp::decode::TelosDecodable;
use crate::types::evm_types::{RawAction, PrintedReceipt, TransferAction, WithdrawAction};
use crate::types::types::NameToAddressCache;

fn make_unique_vrs(block_hash_native: Checksum256, sender_address: Address, trx_index: usize) -> Signature {
    let v = 42u64;
    let hash_biguint = BigUint::from_bytes_be(&block_hash_native.data);
    let trx_index_biguint: BigUint = trx_index.to_biguint().unwrap();
    let r_biguint = hash_biguint + trx_index_biguint;

    let mut s_bytes = [0u8; 32];
    s_bytes[..20].copy_from_slice(sender_address.as_slice());
    let r = U256::from_be_slice(r_biguint.to_bytes_be().as_slice());
    let s = U256::from_be_slice(&s_bytes);
    Signature::from_rs_and_parity(r, s, v).expect("Failed to create signature")
}

#[derive(Clone)]
pub enum Transaction {
    LegacySigned(Signed<TxLegacy>)
}

impl Transaction {
    pub async fn from_raw_action(chain_id: u64, trx_index: usize, block_hash: Checksum256, raw: RawAction, receipt: PrintedReceipt, native_to_evm_cache: &NameToAddressCache) -> Self {
        // TODO: Check for unsigned transactions and handle correctly
        // TODO: Handle 1559
        // TODO: Set trx_index properly for signed and unsigned transactions
        let mut signed_legacy_result = TxLegacy::decode_signed_fields(&mut raw.tx.clone().as_slice());
        if signed_legacy_result.is_err() {
            info!("Failed to decode signed fields for transaction: {:?}", bytes_to_hex(&raw.tx.clone()));
            let unsigned_legacy = TxLegacy::decode_telos(&mut raw.tx.clone().as_slice());
            return Transaction::LegacySigned(unsigned_legacy.unwrap().into_signed(make_unique_vrs(block_hash, Address::ZERO, trx_index)));
        }

        let signed_legacy = signed_legacy_result.unwrap();
        if signed_legacy.signature().r().is_zero() || signed_legacy.signature().s().is_zero() {
            let address = Address::from(raw.sender.expect("Failed to get address from sender in unsigned transaction").data);
            let sig = make_unique_vrs(block_hash, address, trx_index);
            let signed_legacy = signed_legacy.strip_signature().into_signed(sig);
            return Transaction::LegacySigned(signed_legacy);
        }
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

    pub async fn from_withdraw_no_cache(chain_id: u64, trx_index: usize, block_hash: Checksum256, action: WithdrawAction, address: Address) -> Self {
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

    pub async fn from_withdraw(chain_id: u64, trx_index: usize, block_hash: Checksum256, action: WithdrawAction, native_to_evm_cache: &NameToAddressCache) -> Self {
        let address = native_to_evm_cache.get(action.to.n).await.expect("Failed to get address");
        Transaction::from_withdraw_no_cache(chain_id, trx_index, block_hash, action, address).await
    }

    pub fn hash(&self) -> &B256 {
        match self {
            Transaction::LegacySigned(signed_legacy) => {
                signed_legacy.hash()
            }
        }
    }
}