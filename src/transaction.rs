use crate::types::evm_types::EvmRaw;

pub struct Transaction {
    pub raw: Vec<u8>,
}

impl Transaction {
    pub fn from_raw(raw: EvmRaw) -> Self {
        Self {
            raw: raw.tx
        }
    }
}