use alloy_consensus::{Signed, TxLegacy};
use tracing::info;
use crate::block::BasicTrace;
use crate::types::evm_types::{RawAction, PrintedReceipt};

#[derive(Clone)]
pub enum Transaction {
    LegacySigned(Signed<TxLegacy>)
}

impl Transaction {
    pub fn from_raw_action(raw: RawAction, receipt: PrintedReceipt) -> Self {
        let signed_legacy = TxLegacy::decode_signed_fields(&mut raw.tx.clone().as_slice()).unwrap();
        Transaction::LegacySigned(signed_legacy)
    }

    // pub fn from_withdrawal(action: &dyn BasicTrace) -> Self {
    //     info!("hash: {}", signed_legacy.hash());
    //     Transaction::LegacySigned(signed_legacy)
    // }
    //
    // pub fn from_transfer(action: &dyn BasicTrace) -> Self {
    //     info!("hash: {}", signed_legacy.hash());
    //     Transaction::LegacySigned(signed_legacy)
    // }
}