use crate::rlp::telos_rlp_decode::TelosTxDecodable;
use crate::types::evm_types::{PrintedReceipt, RawAction, TransferAction, WithdrawAction};
use crate::types::translator_types::NameToAddressCache;
use alloy::primitives::private::alloy_rlp::Error;
use alloy::primitives::TxKind::Call;
use alloy::primitives::{Address, Bloom, Log, Signature, B256, U256};
use alloy_consensus::{SignableTransaction, TxEnvelope, TxLegacy};
use alloy_rlp::Decodable;
use antelope::chain::checksum::Checksum256;
use num_bigint::{BigUint, ToBigUint};
use reth_primitives::{Receipt, ReceiptWithBloom};

pub fn make_unique_vrs(
    block_hash_native: Checksum256,
    sender_address: Address,
    trx_index: usize,
) -> Signature {
    let v = 42u64;
    let hash_biguint = BigUint::from_bytes_be(&block_hash_native.data);
    let trx_index_biguint: BigUint = trx_index.to_biguint().unwrap();
    let r_biguint = hash_biguint + trx_index_biguint;

    let mut s_bytes = [0x00u8; 32];
    s_bytes[..20].copy_from_slice(sender_address.as_slice());
    let r = U256::from_be_slice(r_biguint.to_bytes_be().as_slice());
    let s = U256::from_be_slice(&s_bytes);
    Signature::from_rs_and_parity(r, s, v).expect("Failed to create signature")
}

#[derive(Clone, Debug)]
pub struct TelosEVMTransaction {
    pub envelope: TxEnvelope,
    pub receipt: PrintedReceipt,
}

impl TelosEVMTransaction {
    pub async fn from_raw_action(
        _chain_id: u64,
        trx_index: usize,
        block_hash: Checksum256,
        raw: RawAction,
        receipt: PrintedReceipt,
    ) -> Result<Self, Error> {
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
                let unsigned_legacy = TxLegacy::decode_telos_signed_fields(
                    &mut raw.tx.clone().as_slice(),
                    Some(sig),
                )?;
                let envelope = TxEnvelope::Legacy(unsigned_legacy);
                return Ok(TelosEVMTransaction { envelope, receipt });
            }

            let signed_legacy = signed_legacy_result.unwrap();
            // Align with contract, if BOTH are zero it's zero and raw.sender is used
            // https://github.com/telosnetwork/telos.evm/blob/9f2024a2a65e7c6b9bb98b36b368c359e24e6885/eosio.evm/include/eosio.evm/transaction.hpp#L205
            if signed_legacy.signature().r().is_zero() && signed_legacy.signature().s().is_zero() {
                let address = Address::from(
                    raw.sender
                        .expect("Failed to get address from sender in unsigned transaction")
                        .data,
                );
                let sig = make_unique_vrs(block_hash, address, trx_index);
                let unsigned_legacy = signed_legacy.strip_signature().into_signed(sig);
                let envelope = TxEnvelope::Legacy(unsigned_legacy);
                return Ok(TelosEVMTransaction { envelope, receipt });
            }

            let envelope = TxEnvelope::Legacy(signed_legacy);
            Ok(TelosEVMTransaction { envelope, receipt })
        } else {
            let type_bit = tx_raw[0];
            match type_bit {
                2 => {
                    let envelope = TxEnvelope::decode(tx_raw).unwrap();
                    Ok(TelosEVMTransaction { envelope, receipt })
                }
                _ => panic!("tx type {} not implemented!", type_bit),
            }
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
        let signed_legacy = tx_legacy.clone().into_signed(sig);
        let mut raw: Vec<u8> = vec![];
        tx_legacy.encode_with_signature_fields(&sig, &mut raw);
        let envelope = TxEnvelope::Legacy(signed_legacy);
        TelosEVMTransaction {
            envelope,
            receipt: PrintedReceipt {
                charged_gas: "".to_string(),
                trx_index: trx_index as u16,
                block: 0,
                status: 1,
                epoch: 0,
                createdaddr: "".to_string(),
                gasused: "5208".to_string(),
                logs: vec![],
                output: "".to_string(),
                errors: None,
            },
        }
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
        let envelope = TxEnvelope::Legacy(signed_legacy);
        TelosEVMTransaction {
            envelope,
            receipt: PrintedReceipt {
                charged_gas: "".to_string(),
                trx_index: trx_index as u16,
                block: 0,
                status: 1,
                epoch: 0,
                createdaddr: "".to_string(),
                gasused: "5208".to_string(),
                logs: vec![],
                output: "".to_string(),
                errors: None,
            },
        }
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
        TelosEVMTransaction::from_withdraw_no_cache(
            chain_id, trx_index, block_hash, action, address,
        )
        .await
    }

    pub fn hash(&self) -> &B256 {
        self.envelope.tx_hash()
    }

    pub fn logs(&self) -> Vec<Log> {
        self.receipt.logs.clone()
    }

    pub fn gas_used(&self) -> U256 {
        self.receipt.gasused.parse().unwrap()
    }

    pub fn receipt(&self, cumulative_gas_used: u64) -> ReceiptWithBloom {
        let tx_gas_used = u64::from_str_radix(&self.receipt.gasused, 16).unwrap();
        let logs = self.receipt.logs.clone();
        let mut bloom = Bloom::default();
        for log in &logs {
            bloom.accrue_log(log);
        }
        let success = self.receipt.status == 1u8;
        ReceiptWithBloom {
            receipt: Receipt {
                tx_type: Default::default(),
                cumulative_gas_used: cumulative_gas_used + tx_gas_used,
                logs,
                success,
            },
            bloom,
        }
    }
}
