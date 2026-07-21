use alloy_primitives::{B256, U256};
use eyre::{bail, Result};
use reth_primitives::Receipt;
use reth_telos_rpc_engine_api::structs::{TelosAccountStateTableRow, TelosAccountTableRow};
use serde::{Deserialize, Serialize};

/// Version of the payload-bound Telos execution metadata protocol.
pub const TELOS_EXECUTION_METADATA_VERSION: u8 = 3;

/// Native execution values effective at a transaction boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelosExecutionContext {
    pub gas_price: U256,
    pub revision: u64,
}

/// Durable link between one native block and an execution payload accepted as VALID.
///
/// Entries are keyed by `native_hash`. The native parent link lets the translator rebuild all
/// retained fork branches after a restart without treating the last accepted payload as canonical.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBranchEntry {
    pub native_block_number: u32,
    pub native_hash: String,
    pub native_parent_hash: Option<String>,
    pub evm_block_number: u32,
    pub evm_hash: B256,
    pub execution_base_fee: U256,
    pub child_context: TelosExecutionContext,
}

/// One native execution value change at a zero-based transaction boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelosExecutionChange<T> {
    pub boundary: u64,
    pub value: T,
}

/// Self-contained execution context bound to one exact execution payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelosExecutionMetadataV3 {
    pub version: u8,
    pub block_hash: B256,
    pub parent_hash: B256,
    pub transaction_count: u64,
    /// EVM block base fee carried by the Engine payload but omitted from the canonical Telos header.
    pub execution_base_fee: U256,
    pub starting_gas_price: U256,
    pub starting_revision: u64,
    pub gas_price_changes: Vec<TelosExecutionChange<U256>>,
    pub revision_changes: Vec<TelosExecutionChange<u64>>,
}

impl TelosExecutionMetadataV3 {
    pub fn new(
        block_hash: B256,
        parent_hash: B256,
        transaction_count: usize,
        execution_base_fee: U256,
        starting_context: TelosExecutionContext,
        gas_price_changes: Vec<(u64, U256)>,
        revision_changes: Vec<(u64, u64)>,
    ) -> Result<Self> {
        let transaction_count = u64::try_from(transaction_count)
            .map_err(|_| eyre::eyre!("transaction count does not fit in u64"))?;
        validate_changes("gas price", &gas_price_changes, transaction_count)?;
        validate_changes("revision", &revision_changes, transaction_count)?;

        Ok(Self {
            version: TELOS_EXECUTION_METADATA_VERSION,
            block_hash,
            parent_hash,
            transaction_count,
            execution_base_fee,
            starting_gas_price: starting_context.gas_price,
            starting_revision: starting_context.revision,
            gas_price_changes: gas_price_changes
                .into_iter()
                .map(|(boundary, value)| TelosExecutionChange { boundary, value })
                .collect(),
            revision_changes: revision_changes
                .into_iter()
                .map(|(boundary, value)| TelosExecutionChange { boundary, value })
                .collect(),
        })
    }

    /// Returns the post-block context inherited by a direct child payload.
    pub fn child_context(&self) -> TelosExecutionContext {
        self.context_at_boundary(self.transaction_count)
    }

    /// Returns the native context effective at an inclusive block boundary.
    pub fn context_at_boundary(&self, boundary: u64) -> TelosExecutionContext {
        let gas_price = self
            .gas_price_changes
            .iter()
            .take_while(|change| change.boundary <= boundary)
            .last()
            .map(|change| change.value)
            .unwrap_or(self.starting_gas_price);
        let revision = self
            .revision_changes
            .iter()
            .take_while(|change| change.boundary <= boundary)
            .last()
            .map(|change| change.value)
            .unwrap_or(self.starting_revision);
        TelosExecutionContext {
            gas_price,
            revision,
        }
    }
}

fn validate_changes<T>(kind: &str, changes: &[(u64, T)], transaction_count: u64) -> Result<()> {
    let mut previous = None;
    for (boundary, _) in changes {
        if *boundary > transaction_count {
            bail!(
                "{kind} change boundary {boundary} exceeds transaction count {transaction_count}"
            );
        }
        if previous.is_some_and(|previous| *boundary <= previous) {
            bail!("{kind} change boundaries must be strictly increasing");
        }
        previous = Some(*boundary);
    }
    Ok(())
}

/// Extra fields accepted by telos-reth-2.
///
/// The legacy scalar execution fields are always `None`; execution semantics are carried only by
/// the versioned, payload-bound sidecar.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelosEngineAPIExtraFields {
    pub statediffs_account: Option<Vec<TelosAccountTableRow>>,
    pub statediffs_accountstate: Option<Vec<TelosAccountStateTableRow>>,
    pub revision_changes: Option<(u64, u64)>,
    pub gasprice_changes: Option<(u64, U256)>,
    pub execution: Option<TelosExecutionMetadataV3>,
    pub new_addresses_using_create: Option<Vec<(u64, U256)>>,
    pub new_addresses_using_openwallet: Option<Vec<(u64, U256)>>,
    pub receipts: Option<Vec<Receipt>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn context(gas_price: u64, revision: u64) -> TelosExecutionContext {
        TelosExecutionContext {
            gas_price: U256::from(gas_price),
            revision,
        }
    }

    #[test]
    fn boundary_zero_applies_to_transaction_zero_and_child() {
        let metadata = TelosExecutionMetadataV3::new(
            B256::repeat_byte(1),
            B256::repeat_byte(2),
            2,
            U256::from(7),
            context(10, 0),
            vec![(0, U256::from(11))],
            vec![(0, 1)],
        )
        .unwrap();

        assert_eq!(metadata.gas_price_changes[0].boundary, 0);
        assert_eq!(metadata.revision_changes[0].boundary, 0);
        assert_eq!(metadata.context_at_boundary(0), context(11, 1));
        assert_eq!(metadata.child_context(), context(11, 1));
    }

    #[test]
    fn boundary_n_is_valid_and_becomes_child_context() {
        let metadata = TelosExecutionMetadataV3::new(
            B256::repeat_byte(1),
            B256::repeat_byte(2),
            2,
            U256::from(7),
            context(10, 0),
            vec![(2, U256::from(12))],
            vec![(2, 3)],
        )
        .unwrap();

        assert_eq!(metadata.context_at_boundary(1), context(10, 0));
        assert_eq!(metadata.context_at_boundary(2), context(12, 3));
        assert_eq!(metadata.child_context(), context(12, 3));
    }

    #[test]
    fn multiple_changes_are_preserved_in_strict_order() {
        let metadata = TelosExecutionMetadataV3::new(
            B256::repeat_byte(1),
            B256::repeat_byte(2),
            3,
            U256::from(7),
            context(10, 0),
            vec![
                (0, U256::from(11)),
                (2, U256::from(12)),
                (3, U256::from(13)),
            ],
            vec![(1, 1), (3, 2)],
        )
        .unwrap();

        assert_eq!(metadata.gas_price_changes.len(), 3);
        assert_eq!(metadata.revision_changes.len(), 2);
        assert_eq!(metadata.child_context(), context(13, 2));
    }

    #[test]
    fn child_context_is_the_next_block_starting_context() {
        let parent = TelosExecutionMetadataV3::new(
            B256::repeat_byte(1),
            B256::repeat_byte(2),
            1,
            U256::from(7),
            context(10, 0),
            vec![(1, U256::from(20))],
            vec![(1, 4)],
        )
        .unwrap();
        let child = TelosExecutionMetadataV3::new(
            B256::repeat_byte(3),
            parent.block_hash,
            0,
            U256::from(7),
            parent.child_context(),
            vec![],
            vec![],
        )
        .unwrap();

        assert_eq!(child.starting_gas_price, U256::from(20));
        assert_eq!(child.starting_revision, 4);
    }

    #[test]
    fn duplicate_or_decreasing_boundaries_are_rejected() {
        let duplicate = TelosExecutionMetadataV3::new(
            B256::ZERO,
            B256::ZERO,
            2,
            U256::from(7),
            context(10, 0),
            vec![(1, U256::from(11)), (1, U256::from(12))],
            vec![],
        );
        assert!(duplicate.is_err());

        let decreasing = TelosExecutionMetadataV3::new(
            B256::ZERO,
            B256::ZERO,
            2,
            U256::from(7),
            context(10, 0),
            vec![],
            vec![(2, 1), (1, 2)],
        );
        assert!(decreasing.is_err());
    }

    #[test]
    fn archived_mainnet_boundary_n_golden_preserves_old_tx_context() {
        // Native block 423,015,053 (EVM 423,015,017) contains two raw transactions followed by
        // doresources. The config change is therefore effective only at the post-block boundary.
        let starting_gas_price = U256::from_str("0x4c5b4d44112").unwrap();
        let child_gas_price = U256::from_str("0x4c5cea1a119").unwrap();
        let metadata = TelosExecutionMetadataV3::new(
            B256::from_str("0x9af24c613ebf3ba3cbd8a29d9b4c24a0cf5589544a162dfe66c98f25a1ce55c0")
                .unwrap(),
            B256::from_str("0x5c9fbb28e4c091a315f59e98063a2a25f8f9ef5413796215cdd4f9744994e292")
                .unwrap(),
            2,
            U256::from(7),
            TelosExecutionContext {
                gas_price: starting_gas_price,
                revision: 1,
            },
            vec![(2, child_gas_price)],
            vec![],
        )
        .unwrap();

        assert_eq!(
            metadata.context_at_boundary(0).gas_price,
            starting_gas_price
        );
        assert_eq!(
            metadata.context_at_boundary(1).gas_price,
            starting_gas_price
        );
        assert_eq!(metadata.context_at_boundary(2).gas_price, child_gas_price);
        assert_eq!(metadata.execution_base_fee, U256::from(7));
        assert_eq!(metadata.child_context().revision, 1);
    }
}
