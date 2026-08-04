// alloy-consensus 0.3 uses the legacy Signature type to retain EIP-155 chain-id parity.
#![allow(deprecated)]

use crate::rlp::telos_rlp_decode::TelosTxDecodable;
use crate::types::evm_types::{PrintedReceipt, RawAction, TransferAction, WithdrawAction};
use crate::types::translator_types::NameToAddressCache;
use alloy_consensus::{SignableTransaction, TxEnvelope, TxLegacy};
use alloy_primitives::TxKind::Call;
use alloy_primitives::{Address, Bloom, Log, Signature, B256, U256};
use alloy_rlp::Decodable;
use antelope::chain::checksum::Checksum256;
use eyre::{eyre, Context, Result};
use reth_primitives::{Receipt, ReceiptWithBloom};

const ADDRESS_HEX_SIZE: usize = 42;

pub fn make_unique_vrs(
    block_hash_native: Checksum256,
    sender_address: Address,
    trx_index: usize,
) -> Result<Signature> {
    let v = 42u64;
    let hash = U256::from_be_slice(&block_hash_native.data);
    let trx_index = u64::try_from(trx_index).wrap_err("transaction index exceeds u64")?;
    let r = hash
        .checked_add(U256::from(trx_index))
        .ok_or_else(|| eyre!("native block hash plus transaction index exceeds U256"))?;

    let mut s_bytes = [0x00u8; 32];
    s_bytes[..20].copy_from_slice(sender_address.as_slice());
    let s = U256::from_be_slice(&s_bytes);
    Signature::from_rs_and_parity(r, s, v)
        .map_err(|error| eyre!("failed to create synthetic Telos signature: {error}"))
}

fn native_asset_value(amount: i64) -> Result<U256> {
    let amount = u64::try_from(amount).wrap_err("native asset amount cannot be negative")?;
    U256::from(amount)
        .checked_mul(U256::from(100_000_000_000_000u64))
        .ok_or_else(|| eyre!("native asset amount overflows U256 wei value"))
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
    ) -> Result<Self> {
        // TODO: Check for unsigned transactions and handle correctly
        // TODO: Set trx_index properly for signed and unsigned transactions
        let type_bit = *raw
            .tx
            .first()
            .ok_or_else(|| eyre!("raw EVM transaction is empty"))?;
        let mut tx_raw = raw.tx.as_slice();

        if (0xc0..=0xfe).contains(&type_bit) {
            let mut signed_legacy_result = TxLegacy::decode_signed_fields(&mut tx_raw);
            // If we fail to decode with the strict RLP from reth,
            // and we don't have a raw.sender which suggests a native signed trx
            // then try the telos legacy decode without passing a signature
            if signed_legacy_result.is_err() && raw.sender.is_none() {
                let mut telos_tx_raw = raw.tx.as_slice();
                signed_legacy_result =
                    TxLegacy::decode_telos_signed_fields(&mut telos_tx_raw, None);
            }

            let signed_legacy = if let Ok(signed_legacy) = signed_legacy_result {
                signed_legacy
            } else {
                let sender = raw.sender.as_ref().ok_or_else(|| {
                    eyre!("unsigned Telos transaction is missing its native sender")
                })?;
                let address = Address::from(sender.data);
                let sig = make_unique_vrs(block_hash, address, trx_index)?;
                let mut unsigned_tx_raw = raw.tx.as_slice();
                TxLegacy::decode_telos_signed_fields(&mut unsigned_tx_raw, Some(sig))
                    .wrap_err("failed to decode unsigned Telos legacy transaction")?
            };

            // Align with contract, if BOTH are zero it's zero and raw.sender is used
            // https://github.com/telosnetwork/telos.evm/blob/9f2024a2a65e7c6b9bb98b36b368c359e24e6885/eosio.evm/include/eosio.evm/transaction.hpp#L205
            if signed_legacy.signature().r().is_zero() && signed_legacy.signature().s().is_zero() {
                let sender = raw.sender.as_ref().ok_or_else(|| {
                    eyre!("zero-signature Telos transaction is missing its native sender")
                })?;
                let address = Address::from(sender.data);
                let sig = make_unique_vrs(block_hash, address, trx_index)?;
                let unsigned_legacy = signed_legacy.strip_signature().into_signed(sig);
                let envelope = TxEnvelope::Legacy(unsigned_legacy);
                return Ok(TelosEVMTransaction { envelope, receipt });
            }

            let envelope = TxEnvelope::Legacy(signed_legacy);
            Ok(TelosEVMTransaction { envelope, receipt })
        } else {
            match type_bit {
                2 => {
                    let envelope = TxEnvelope::decode(&mut tx_raw)
                        .wrap_err("failed to decode EIP-1559 Telos transaction")?;
                    if !tx_raw.is_empty() {
                        return Err(eyre!(
                            "EIP-1559 Telos transaction has {} trailing bytes",
                            tx_raw.len()
                        ));
                    }
                    if !matches!(envelope, TxEnvelope::Eip1559(_)) {
                        return Err(eyre!(
                            "type-2 Telos transaction decoded as another envelope"
                        ));
                    }
                    Ok(TelosEVMTransaction { envelope, receipt })
                }
                _ => Err(eyre!("Telos transaction type {type_bit} is unsupported")),
            }
        }
    }

    pub async fn from_transfer(
        chain_id: u64,
        trx_index: usize,
        block_hash: Checksum256,
        action: TransferAction,
        native_to_evm_cache: &NameToAddressCache,
    ) -> eyre::Result<Self> {
        let str_memo = String::from_utf8(action.memo.clone());
        let address: Address = match str_memo {
            Ok(memo) if memo.len() == ADDRESS_HEX_SIZE && memo.starts_with("0x") => memo
                .parse()
                .wrap_err("transfer memo contains an invalid EVM address")?,
            _ => native_to_evm_cache.get(action.from.n).await?,
        };

        let value = native_asset_value(action.quantity.amount())?;
        let receipt_index =
            u16::try_from(trx_index).wrap_err("synthetic transfer index exceeds u16")?;

        let tx_legacy = TxLegacy {
            chain_id: Some(chain_id),
            nonce: 0,
            gas_price: 0,
            gas_limit: 21_000,
            to: Call(address),
            value,
            input: Default::default(),
        };

        let sig = make_unique_vrs(block_hash, Address::ZERO, trx_index)?;
        let signed_legacy = tx_legacy.clone().into_signed(sig);
        let mut raw: Vec<u8> = vec![];
        tx_legacy.encode_with_signature_fields(&sig, &mut raw);
        let envelope = TxEnvelope::Legacy(signed_legacy);
        Ok(TelosEVMTransaction {
            envelope,
            receipt: PrintedReceipt {
                charged_gas: "".to_string(),
                trx_index: receipt_index,
                block: 0,
                status: 1,
                epoch: 0,
                createdaddr: "".to_string(),
                gasused: "5208".to_string(),
                logs: vec![],
                output: "".to_string(),
                errors: None,
            },
        })
    }

    pub async fn from_withdraw_no_cache(
        chain_id: u64,
        trx_index: usize,
        block_hash: Checksum256,
        action: WithdrawAction,
        address: Address,
    ) -> Result<Self> {
        let value = native_asset_value(action.quantity.amount())?;
        let receipt_index =
            u16::try_from(trx_index).wrap_err("synthetic withdrawal index exceeds u16")?;
        let tx_legacy = TxLegacy {
            chain_id: Some(chain_id),
            nonce: 0,
            gas_price: 0,
            gas_limit: 21_000,
            to: Call(Address::ZERO),
            value,
            input: Default::default(),
        };

        let sig = make_unique_vrs(block_hash, address, trx_index)?;
        let signed_legacy = tx_legacy.into_signed(sig);
        let envelope = TxEnvelope::Legacy(signed_legacy);
        Ok(TelosEVMTransaction {
            envelope,
            receipt: PrintedReceipt {
                charged_gas: "".to_string(),
                trx_index: receipt_index,
                block: 0,
                status: 1,
                epoch: 0,
                createdaddr: "".to_string(),
                gasused: "5208".to_string(),
                logs: vec![],
                output: "".to_string(),
                errors: None,
            },
        })
    }

    pub async fn from_withdraw(
        chain_id: u64,
        trx_index: usize,
        block_hash: Checksum256,
        action: WithdrawAction,
        native_to_evm_cache: &NameToAddressCache,
    ) -> eyre::Result<Self> {
        let address = native_to_evm_cache.get(action.to.n).await?;

        let telos_evm_transactions = TelosEVMTransaction::from_withdraw_no_cache(
            chain_id, trx_index, block_hash, action, address,
        )
        .await?;

        Ok(telos_evm_transactions)
    }

    pub fn hash(&self) -> &B256 {
        self.envelope.tx_hash()
    }

    pub fn logs(&self) -> Vec<Log> {
        self.receipt.logs.clone()
    }

    pub fn gas_used(&self) -> Result<U256> {
        let encoded = self
            .receipt
            .gasused
            .strip_prefix("0x")
            .unwrap_or(&self.receipt.gasused);
        U256::from_str_radix(encoded, 16).wrap_err("receipt gasused is not hexadecimal U256")
    }

    pub fn receipt(&self, cumulative_gas_used: u64) -> Result<ReceiptWithBloom> {
        let encoded = self
            .receipt
            .gasused
            .strip_prefix("0x")
            .unwrap_or(&self.receipt.gasused);
        let tx_gas_used =
            u64::from_str_radix(encoded, 16).wrap_err("receipt gasused is not hexadecimal u64")?;
        let cumulative_gas_used = cumulative_gas_used
            .checked_add(tx_gas_used)
            .ok_or_else(|| eyre!("receipt cumulative gas overflows u64"))?;
        if !matches!(self.receipt.status, 0 | 1) {
            return Err(eyre!(
                "receipt status {} is not 0 or 1",
                self.receipt.status
            ));
        }
        let logs = self.receipt.logs.clone();
        let mut bloom = Bloom::default();
        for log in &logs {
            bloom.accrue_log(log);
        }
        let success = self.receipt.status == 1u8;
        Ok(ReceiptWithBloom {
            receipt: Receipt {
                tx_type: Default::default(),
                cumulative_gas_used,
                logs,
                success,
            },
            bloom,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checksum(byte: u8) -> Checksum256 {
        Checksum256::from_bytes(&[byte; 32]).unwrap()
    }

    fn transaction_with_receipt(receipt: PrintedReceipt) -> TelosEVMTransaction {
        let transaction = TxLegacy {
            chain_id: Some(40),
            nonce: 0,
            gas_price: 0,
            gas_limit: 21_000,
            to: Call(Address::ZERO),
            value: U256::ZERO,
            input: Default::default(),
        };
        let signature = make_unique_vrs(checksum(1), Address::ZERO, 0).unwrap();
        TelosEVMTransaction {
            envelope: TxEnvelope::Legacy(transaction.into_signed(signature)),
            receipt,
        }
    }

    #[tokio::test]
    async fn malformed_and_unsupported_raw_transactions_are_errors() {
        for tx in [vec![], vec![1], vec![2]] {
            let result = TelosEVMTransaction::from_raw_action(
                40,
                0,
                checksum(1),
                RawAction {
                    tx,
                    ..Default::default()
                },
                PrintedReceipt::default(),
            )
            .await;
            assert!(result.is_err());
        }
    }

    #[test]
    fn synthetic_signature_rejects_u256_overflow() {
        assert!(make_unique_vrs(checksum(u8::MAX), Address::ZERO, 1).is_err());
    }

    #[test]
    fn malformed_receipt_gas_and_status_are_errors() {
        let invalid_gas = transaction_with_receipt(PrintedReceipt {
            gasused: "not-hex".to_string(),
            status: 1,
            ..Default::default()
        });
        assert!(invalid_gas.gas_used().is_err());
        assert!(invalid_gas.receipt(0).is_err());

        let invalid_status = transaction_with_receipt(PrintedReceipt {
            gasused: "0".to_string(),
            status: 2,
            ..Default::default()
        });
        assert!(invalid_status.receipt(0).is_err());

        let overflow = transaction_with_receipt(PrintedReceipt {
            gasused: "1".to_string(),
            status: 1,
            ..Default::default()
        });
        assert!(overflow.receipt(u64::MAX).is_err());
    }

    #[test]
    fn negative_native_asset_amount_is_rejected() {
        assert!(native_asset_value(-1).is_err());
    }
}
