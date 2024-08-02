use crate::rlp::alloy_rlp::TelosTxDecodable;
use crate::types::evm_types::{PrintedReceipt, RawAction, TransferAction, WithdrawAction};
use crate::types::translator_types::NameToAddressCache;
use alloy::primitives::TxKind::Call;
use alloy::primitives::{Address, Log, Signature, B256, U256};
use alloy_consensus::{SignableTransaction, Signed, TxLegacy};
use antelope::chain::checksum::Checksum256;
use num_bigint::{BigUint, ToBigUint};

pub fn make_unique_vrs(
    block_hash_native: Checksum256,
    sender_address: Address,
    trx_index: usize,
) -> Signature {
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
    LegacySigned(Signed<TxLegacy>, Option<PrintedReceipt>),
}

impl Transaction {
    pub async fn from_raw_action(
        _chain_id: u64,
        trx_index: usize,
        block_hash: Checksum256,
        raw: RawAction,
        receipt: PrintedReceipt,
    ) -> Self {
        // TODO: Check for unsigned transactions and handle correctly
        // TODO: Set trx_index properly for signed and unsigned transactions
        let tx_raw = &mut raw.tx.as_slice();

        if tx_raw[0] >= 0xc0 && tx_raw[0] <= 0xfe {
            let signed_legacy_result = TxLegacy::decode_signed_fields(tx_raw);
            if signed_legacy_result.is_err() {
                let address = Address::from(
                    raw.sender
                        .expect("Failed to get address from sender in unsigned transaction")
                        .data,
                );
                let sig = make_unique_vrs(block_hash, address, trx_index);
                let unsigned_legacy =
                    TxLegacy::decode_telos_signed_fields(&mut raw.tx.clone().as_slice(), sig);
                return Transaction::LegacySigned(unsigned_legacy.unwrap(), Some(receipt));
            }

            let signed_legacy = signed_legacy_result.unwrap();
            // Align with contract, if BOTH are zero it's zero and raw.sender is used
            //   https://github.com/telosnetwork/telos.evm/blob/9f2024a2a65e7c6b9bb98b36b368c359e24e6885/eosio.evm/include/eosio.evm/transaction.hpp#L205
            if signed_legacy.signature().r().is_zero() && signed_legacy.signature().s().is_zero() {
                let address = Address::from(
                    raw.sender
                        .expect("Failed to get address from sender in unsigned transaction")
                        .data,
                );
                let sig = make_unique_vrs(block_hash, address, trx_index);
                let unsigned_legacy = signed_legacy.strip_signature().into_signed(sig);
                return Transaction::LegacySigned(unsigned_legacy, Some(receipt));
            }

            Transaction::LegacySigned(signed_legacy, Some(receipt))
        } else {
            // TODO: Handle other tx types
            panic!("Other tx types other than legacy not implemented yet!");
        }
    }

    pub async fn from_transfer(
        chain_id: u64,
        trx_index: usize,
        block_hash: Checksum256,
        action: TransferAction,
        native_to_evm_cache: &NameToAddressCache,
    ) -> Self {
        let address: Address = if action.memo.starts_with("0x") {
            action.memo.parse().unwrap()
        } else {
            native_to_evm_cache
                .get(action.from.n)
                .await
                .expect("Failed to get address")
        };

        let value = U256::from(action.quantity.amount()) * U256::from(100_000_000_000_000i64);

        let tx_legacy = TxLegacy {
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
        Transaction::LegacySigned(signed_legacy, None)
    }

    pub async fn from_withdraw_no_cache(
        chain_id: u64,
        trx_index: usize,
        block_hash: Checksum256,
        action: WithdrawAction,
        address: Address,
    ) -> Self {
        let value = U256::from(action.quantity.amount()) * U256::from(100_000_000_000_000i64);
        let tx_legacy = TxLegacy {
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
        Transaction::LegacySigned(signed_legacy, None)
    }

    pub async fn from_withdraw(
        chain_id: u64,
        trx_index: usize,
        block_hash: Checksum256,
        action: WithdrawAction,
        native_to_evm_cache: &NameToAddressCache,
    ) -> Self {
        let address = native_to_evm_cache
            .get(action.to.n)
            .await
            .expect("Failed to get address");
        Transaction::from_withdraw_no_cache(chain_id, trx_index, block_hash, action, address).await
    }

    pub fn hash(&self) -> &B256 {
        match self {
            Transaction::LegacySigned(signed_legacy, _receipt) => signed_legacy.hash(),
        }
    }

    pub fn logs(&self) -> Vec<Log> {
        match self {
            Transaction::LegacySigned(_, Some(receipt)) => receipt.logs.clone(),
            _ => vec![],
        }
    }

    pub fn gas_used(&self) -> U256 {
        match self {
            Transaction::LegacySigned(_, Some(receipt)) => {
                receipt.gasused.parse().expect("Failed to parse gas used")
            }
            _ => U256::ZERO,
        }
    }
}
