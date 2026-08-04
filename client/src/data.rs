use eyre::{eyre, Context};
use rocksdb::{DBWithThreadMode, SingleThreaded, WriteBatch, WriteOptions, DB};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Debug};
use std::{fs, path::Path, sync::Arc};
use telos_translator_rs::block::TelosEVMBlock;
use telos_translator_rs::types::execution_metadata::{
    ExecutionBranchEntry, TELOS_EXECUTION_METADATA_VERSION,
};
use tracing::info;

use crate::client::Error;

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Block {
    pub number: u32,
    pub hash: String,
}

/// Last payload made canonical by an exact VALID forkchoice response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCheckpoint {
    pub version: u8,
    pub branch: ExecutionBranchEntry,
}

impl TryFrom<&TelosEVMBlock> for ExecutionCheckpoint {
    type Error = Error;

    fn try_from(value: &TelosEVMBlock) -> Result<Self, Self::Error> {
        Ok(Self {
            version: 3,
            branch: execution_branch_entry(value)?,
        })
    }
}

impl From<ExecutionBranchEntry> for ExecutionCheckpoint {
    fn from(branch: ExecutionBranchEntry) -> Self {
        Self { version: 3, branch }
    }
}

pub fn execution_branch_entry(value: &TelosEVMBlock) -> Result<ExecutionBranchEntry, Error> {
    let metadata = value.extra_fields.execution.as_ref().ok_or_else(|| {
        Error::Database(eyre!(
            "accepted block {} is missing execution metadata",
            value.block_num
        ))
    })?;
    if metadata.version != TELOS_EXECUTION_METADATA_VERSION {
        return Err(Error::Database(eyre!(
            "accepted block {} has unsupported execution metadata version {}",
            value.block_num,
            metadata.version
        )));
    }
    let transaction_count =
        u64::try_from(value.execution_payload.transactions.len()).map_err(|_| {
            Error::Database(eyre!(
                "accepted block transaction count does not fit in u64"
            ))
        })?;
    if metadata.block_hash != value.block_hash
        || metadata.parent_hash != value.header.parent_hash
        || metadata.transaction_count != transaction_count
        || value.execution_payload.block_hash != value.block_hash
        || value.execution_payload.block_number != u64::from(value.block_num)
        || value.execution_payload.parent_hash != value.header.parent_hash
    {
        return Err(Error::Database(eyre!(
            "accepted block {} has execution metadata bound to another payload",
            value.block_num
        )));
    }
    if metadata.execution_base_fee != value.execution_payload.base_fee_per_gas {
        return Err(Error::Database(eyre!(
            "accepted block {} execution base fee metadata {} does not match payload {}",
            value.block_num,
            metadata.execution_base_fee,
            value.execution_payload.base_fee_per_gas
        )));
    }
    Ok(ExecutionBranchEntry {
        native_block_number: value.ship_block_num,
        native_hash: value.ship_hash.clone(),
        native_parent_hash: value.ship_parent_hash.clone(),
        evm_block_number: value.block_num,
        evm_hash: value.block_hash,
        execution_base_fee: metadata.execution_base_fee,
        child_context: metadata.child_context(),
    })
}

pub struct Lib<'a>(pub &'a TelosEVMBlock);

impl Debug for Lib<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "number: {}, hash: {}", self.0.lib_num, self.0.lib_hash)
    }
}

impl From<&TelosEVMBlock> for Block {
    fn from(value: &TelosEVMBlock) -> Self {
        Block {
            number: value.block_num,
            hash: value.block_hash.to_string(),
        }
    }
}

impl From<Lib<'_>> for Block {
    fn from(Lib(value): Lib) -> Self {
        Block {
            number: value.lib_num,
            hash: value.lib_hash.to_string(),
        }
    }
}

#[derive(Clone)]
pub struct Database {
    db: Arc<DBWithThreadMode<SingleThreaded>>,
}

impl Database {
    const EXECUTION_BRANCH_PREFIX: &'static str = "execution_branch:v3:";

    fn block_key(number: u32) -> String {
        format!("blocks:{number:020}")
    }

    fn execution_branch_key(native_hash: &str) -> String {
        format!("{}{native_hash}", Self::EXECUTION_BRANCH_PREFIX)
    }

    fn durable_write_options() -> WriteOptions {
        let mut options = WriteOptions::default();
        options.set_sync(true);
        options
    }

    pub fn open(path: &str) -> Result<Self, Error> {
        Ok(Database {
            db: Arc::new(
                DB::open_default(path)
                    .wrap_err("Failed to open database for given path")
                    .map_err(Error::Database)?,
            ),
        })
    }

    pub fn init(path: &str) -> Result<Self, Error> {
        if Path::new(path).exists() {
            fs::remove_dir_all(path)
                .map_err(|error| eyre!("Failed to delete data dir {path}. {error}"))
                .map_err(Error::Database)?;
            info!("Data dir {path} deleted.");
        }
        Self::open(path)
    }

    pub fn put_block(&self, block: Block) -> Result<(), Error> {
        let value = serde_json::to_string(&block)
            .wrap_err("Failed to serialize block")
            .map_err(Error::Database)?;

        self.db
            .put(Self::block_key(block.number), value)
            .wrap_err("Failed to put block into database")
            .map_err(Error::Database)
    }

    pub fn delete_block(&self, number: u32) -> Result<(), Error> {
        self.db
            .delete(Self::block_key(number))
            .wrap_err("Failed to delete block from database")
            .map_err(Error::Database)
    }

    pub fn put_lib(&self, lib: Block) -> Result<(), Error> {
        let value = serde_json::to_string(&lib)
            .wrap_err("Failed to serialize lib")
            .map_err(Error::Database)?;

        self.db
            .put_opt("lib", value, &Self::durable_write_options())
            .wrap_err("Failed to put lib into database")
            .map_err(Error::Database)
    }

    pub fn put_execution_checkpoint(&self, checkpoint: ExecutionCheckpoint) -> Result<(), Error> {
        let value = serde_json::to_vec(&checkpoint)
            .wrap_err("Failed to serialize execution checkpoint")
            .map_err(Error::Database)?;
        self.db
            .put_opt(
                "canonical_execution_checkpoint:v3",
                value,
                &Self::durable_write_options(),
            )
            .wrap_err("Failed to put execution checkpoint into database")
            .map_err(Error::Database)
    }

    pub fn get_execution_checkpoint(&self) -> Result<Option<ExecutionCheckpoint>, Error> {
        let checkpoint = self
            .db
            .get("canonical_execution_checkpoint:v3")
            .map_err(|error| eyre!("Cannot get execution checkpoint: {error}"))
            .map_err(Error::Database)?
            .map(|value| serde_json::from_slice(&value))
            .transpose()
            .map_err(|error| eyre!("Cannot parse execution checkpoint JSON: {error}"))
            .map_err(Error::Database)?;
        if checkpoint
            .as_ref()
            .is_some_and(|checkpoint: &ExecutionCheckpoint| checkpoint.version != 3)
        {
            return Err(Error::Database(eyre!(
                "unsupported canonical execution checkpoint version"
            )));
        }
        Ok(checkpoint)
    }

    pub fn put_execution_branch_entry(&self, entry: &ExecutionBranchEntry) -> Result<(), Error> {
        let key = Self::execution_branch_key(&entry.native_hash);
        if let Some(existing) = self
            .db
            .get(&key)
            .map_err(|error| Error::Database(error.into()))?
        {
            let existing: ExecutionBranchEntry = serde_json::from_slice(&existing)
                .wrap_err("Cannot parse execution branch entry JSON")
                .map_err(Error::Database)?;
            if &existing != entry {
                return Err(Error::Database(eyre!(
                    "native block {} maps to conflicting execution branches",
                    entry.native_hash
                )));
            }
            return Ok(());
        }

        let value = serde_json::to_vec(entry)
            .wrap_err("Failed to serialize execution branch entry")
            .map_err(Error::Database)?;
        self.db
            .put_opt(key, value, &Self::durable_write_options())
            .wrap_err("Failed to put execution branch entry into database")
            .map_err(Error::Database)
    }

    pub fn get_execution_branch_entries(&self) -> Result<Vec<ExecutionBranchEntry>, Error> {
        let mut iterator = self.db.raw_iterator();
        iterator.seek(Self::EXECUTION_BRANCH_PREFIX.as_bytes());
        let mut entries: Vec<ExecutionBranchEntry> = Vec::new();
        while iterator.valid() {
            let Some(key) = iterator.key() else {
                break;
            };
            if !key.starts_with(Self::EXECUTION_BRANCH_PREFIX.as_bytes()) {
                break;
            }
            let value = iterator.value().ok_or_else(|| {
                Error::Database(eyre!("execution branch database entry has no value"))
            })?;
            entries.push(
                serde_json::from_slice(value)
                    .wrap_err("Cannot parse execution branch entry JSON")
                    .map_err(Error::Database)?,
            );
            iterator.next();
        }
        iterator
            .status()
            .map_err(|error| Error::Database(error.into()))?;
        entries.sort_by(|left, right| {
            left.native_block_number
                .cmp(&right.native_block_number)
                .then_with(|| left.native_hash.cmp(&right.native_hash))
        });
        Ok(entries)
    }

    pub fn get_execution_branch_entry(
        &self,
        native_hash: &str,
    ) -> Result<Option<ExecutionBranchEntry>, Error> {
        self.db
            .get(Self::execution_branch_key(native_hash))
            .map_err(|error| Error::Database(error.into()))?
            .map(|value| serde_json::from_slice(&value))
            .transpose()
            .wrap_err("Cannot parse execution branch entry JSON")
            .map_err(Error::Database)
    }

    /// Removes only branch records at or below the exact irreversible native block.
    ///
    /// The exact LIB entry is retained as the parent anchor. All entries above LIB remain available
    /// for a later forkchoice, even when they are not on the current preferred branch.
    pub fn prune_execution_branches(
        &self,
        irreversible_block: u32,
        irreversible_hash: &str,
        recovery_window: u32,
    ) -> Result<usize, Error> {
        let entries = self.get_execution_branch_entries()?;
        if entries.iter().any(|entry| {
            entry.native_hash == irreversible_hash
                && entry.native_block_number != irreversible_block
        }) {
            return Err(Error::Database(eyre!(
                "irreversible native hash {irreversible_hash} has a conflicting block number"
            )));
        }

        let mut batch = WriteBatch::default();
        let mut removed = 0;
        for entry in entries {
            if should_prune_execution_branch(
                &entry,
                irreversible_block,
                irreversible_hash,
                recovery_window,
            ) {
                batch.delete(Self::execution_branch_key(&entry.native_hash));
                removed += 1;
            }
        }
        if removed > 0 {
            self.db
                .write_opt(batch, &Self::durable_write_options())
                .wrap_err("Failed to prune irreversible execution branch entries")
                .map_err(Error::Database)?;
        }
        Ok(removed)
    }

    pub fn get_block_or_prev(&self, number: u32) -> Result<Option<Block>, Error> {
        let mut iter = self.db.raw_iterator();

        iter.seek_for_prev(Self::block_key(number));

        if !iter.valid() {
            return Ok(None);
        }

        let Some(value) = iter.value() else {
            return Ok(None);
        };

        serde_json::from_slice(value)
            .map(Some)
            .map_err(|error| eyre!("Cannot parse block JSON: {error}"))
            .map_err(Error::Database)
    }

    /// Returns only the block stored at `number`; it never substitutes an earlier checkpoint.
    pub fn get_block(&self, number: u32) -> Result<Option<Block>, Error> {
        self.db
            .get(Self::block_key(number))
            .map_err(|error| eyre!("Cannot get block {number}: {error}"))
            .map_err(Error::Database)?
            .map(|value| serde_json::from_slice(&value))
            .transpose()
            .map_err(|error| eyre!("Cannot parse block {number} JSON: {error}"))
            .map_err(Error::Database)
    }

    pub fn get_lib(&self) -> Result<Option<Block>, Error> {
        self.db
            .get("lib")
            .map_err(|error| eyre!("Cannot get lib: {error}"))
            .map_err(Error::Database)?
            .map(|value| serde_json::from_slice(&value))
            .transpose()
            .map_err(|error| eyre!("Cannot parse lib JSON: {error}"))
            .map_err(Error::Database)
    }
}

fn should_prune_execution_branch(
    entry: &ExecutionBranchEntry,
    irreversible_block: u32,
    irreversible_hash: &str,
    recovery_window: u32,
) -> bool {
    if entry.native_block_number == irreversible_block && entry.native_hash != irreversible_hash {
        return true;
    }

    // Reth can acknowledge a canonical forkchoice before its latest database pages reach durable
    // storage. Retain the same recent window as the translator block database so a restart after
    // abrupt power loss can bind the execution client's recovered head to previously VALID
    // metadata instead of either guessing its context or becoming permanently unrecoverable.
    entry.native_block_number < irreversible_block.saturating_sub(recovery_window)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{B256, U256};
    use telos_translator_rs::types::execution_metadata::TelosExecutionContext;

    fn branch(native_number: u32, native_hash: &str, byte: u8) -> ExecutionBranchEntry {
        ExecutionBranchEntry {
            native_block_number: native_number,
            native_hash: native_hash.to_string(),
            native_parent_hash: Some("parent".to_string()),
            evm_block_number: native_number.saturating_sub(36),
            evm_hash: B256::repeat_byte(byte),
            execution_base_fee: U256::from(7),
            child_context: TelosExecutionContext {
                gas_price: U256::from(byte),
                revision: u64::from(byte),
            },
        }
    }

    fn temporary_database_path(test_name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "telos-consensus-{test_name}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ))
    }

    #[test]
    fn durable_branch_entries_and_checkpoint_survive_reopen() {
        let path = temporary_database_path("restart");
        let branch_a = branch(100, "native-a", 1);
        let branch_b = branch(100, "native-b", 2);
        let checkpoint = ExecutionCheckpoint::from(branch_a.clone());
        {
            let database = Database::open(path.to_str().unwrap()).unwrap();
            database.put_execution_branch_entry(&branch_a).unwrap();
            database.put_execution_branch_entry(&branch_b).unwrap();
            database
                .put_execution_checkpoint(checkpoint.clone())
                .unwrap();
        }

        {
            let database = Database::open(path.to_str().unwrap()).unwrap();
            assert_eq!(
                database.get_execution_branch_entries().unwrap(),
                vec![branch_a, branch_b]
            );
            assert_eq!(
                database.get_execution_checkpoint().unwrap(),
                Some(checkpoint)
            );
        }
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn pruning_never_removes_branches_above_lib() {
        let path = temporary_database_path("pruning");
        let old = branch(99, "old", 1);
        let lib = branch(100, "lib", 2);
        let side_at_lib = branch(100, "side-at-lib", 3);
        let newer_a = branch(101, "newer-a", 4);
        let newer_b = branch(101, "newer-b", 5);
        {
            let database = Database::open(path.to_str().unwrap()).unwrap();
            for entry in [&old, &lib, &side_at_lib, &newer_a, &newer_b] {
                database.put_execution_branch_entry(entry).unwrap();
            }
            assert_eq!(database.prune_execution_branches(100, "lib", 0).unwrap(), 2);
            assert_eq!(
                database.get_execution_branch_entries().unwrap(),
                vec![lib, newer_a, newer_b]
            );
        }
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn pruning_retains_a_durable_execution_recovery_window() {
        let path = temporary_database_path("recovery-window");
        let too_old = branch(89, "too-old", 1);
        let recovered_head = branch(90, "recovered-head", 2);
        let recent = branch(99, "recent", 3);
        let lib = branch(100, "lib", 4);
        let side_at_lib = branch(100, "side-at-lib", 5);
        {
            let database = Database::open(path.to_str().unwrap()).unwrap();
            for entry in [&too_old, &recovered_head, &recent, &lib, &side_at_lib] {
                database.put_execution_branch_entry(entry).unwrap();
            }
            assert_eq!(
                database.prune_execution_branches(100, "lib", 10).unwrap(),
                2
            );
            assert_eq!(
                database.get_execution_branch_entries().unwrap(),
                vec![recovered_head, recent, lib]
            );
        }
        fs::remove_dir_all(path).unwrap();
    }
}
