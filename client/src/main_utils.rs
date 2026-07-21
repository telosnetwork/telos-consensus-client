use std::cmp;

use crate::client::Error::CannotStartConsensusClient;
use crate::client::{ConsensusClient, Error, Shutdown};
use crate::config::{AppConfig, CliArgs};
use crate::data::ExecutionCheckpoint;
use antelope::chain::checksum::Checksum256;
use eyre::eyre;
use reth_primitives::B256;
use std::str::FromStr;
use telos_translator_rs::block::TelosEVMBlock;
use telos_translator_rs::translator::Translator;
use telos_translator_rs::types::execution_metadata::ExecutionBranchEntry;
use tokio::sync::mpsc;
use tracing::level_filters::LevelFilter;
use tracing::{info, warn};

pub async fn run_client(args: CliArgs, config: AppConfig) -> Result<Shutdown, Error> {
    let client = build_consensus_client(&args, config).await?;
    let client_shutdown = client.shutdown_handle();

    let translator = Translator::new((&client.config).into());
    let translator_shutdown = translator.shutdown_handle();

    info!(
        latest_finalized_executor_block = ?client.latest_finalized_executor_block,
        latest_executor_block = ?client.latest_executor_block,
        "Telos consensus client starting, awaiting result..."
    );

    let (block_sender, block_receiver) = mpsc::channel::<TelosEVMBlock>(1000);

    let client_handle = tokio::spawn(client.run(block_receiver));

    info!(
        evm_start_block = translator.config.evm_start_block,
        evm_stop_block = ?translator.config.evm_stop_block,
        "Telos translator client launching, awaiting result...",
    );

    let translator_handle = tokio::spawn(translator.launch(Some(block_sender)));

    let client_error = client_handle
        .await
        .map_err(From::from)
        .and_then(|inner| inner)
        .err();

    if let Some(error) = client_error.as_ref() {
        warn!("Consensus client run failed! Error: {error:#}");
    }

    if !translator_shutdown.is_finished() {
        if let Err(error) = translator_shutdown.shutdown().await {
            warn!("Cannot send shutdown signal! Error: {error:#}");
        }
    }

    let translator_error = translator_handle
        .await
        .map_err(From::from)
        .and_then(|inner| inner)
        .map_err(|error| Error::TranslatorError(error.to_string()))
        .err();

    if let Some(error) = translator_error.as_ref() {
        warn!("{error:#}");
    }

    if let Some(error) = client_error.or(translator_error) {
        return Err(error);
    }

    info!("Reached stop block/signal, consensus client run finished!");
    Ok(client_shutdown)
}

pub async fn build_consensus_client(
    args: &CliArgs,
    config: AppConfig,
) -> Result<ConsensusClient, Error> {
    if !matches!(config.chain_id.0, 40 | 41) {
        return Err(Error::CannotStartConsensusClient(format!(
            "Unsupported Telos EVM chain id {}",
            config.chain_id.0
        )));
    }
    if config.batch_size == 0 {
        return Err(Error::CannotStartConsensusClient(
            "Batch size must be greater than zero".to_string(),
        ));
    }
    if config.block_checkpoint_interval == 0 {
        return Err(Error::CannotStartConsensusClient(
            "Block checkpoint interval must be greater than zero".to_string(),
        ));
    }
    if config.execution_connect_timeout_ms == Some(0)
        || config.execution_request_timeout_ms == Some(0)
        || config.native_request_timeout_ms == Some(0)
        || config.execution_max_response_bytes == Some(0)
    {
        return Err(Error::CannotStartConsensusClient(
            "Execution timeouts and response-size limit must be greater than zero".to_string(),
        ));
    }
    if config.evm_start_block > config.evm_stop_block.unwrap_or(u32::MAX) {
        return Err(Error::CannotStartConsensusClient(
            "Start block is after stop block".to_string(),
        ));
    }
    if config.execution_context_anchor_block != config.evm_start_block {
        return Err(Error::CannotStartConsensusClient(format!(
            "Execution context anchor block {} must match EVM start block {}",
            config.execution_context_anchor_block, config.evm_start_block
        )));
    }
    let expected_start = config
        .execution_anchor_block_number
        .checked_add(1)
        .ok_or_else(|| {
            Error::CannotStartConsensusClient(
                "Execution anchor block number cannot have a child".to_string(),
            )
        })?;
    if expected_start != u64::from(config.evm_start_block) {
        return Err(Error::CannotStartConsensusClient(format!(
            "EVM start block {} must be the child of execution anchor {}",
            config.evm_start_block, config.execution_anchor_block_number
        )));
    }
    parse_expected_first_child_hash(&config.validate_hash)?;

    let execution_anchor_hash =
        B256::from_str(&config.execution_anchor_block_hash).map_err(|_| {
            Error::CannotStartConsensusClient(
                "Execution anchor hash must be exactly 32 bytes of hexadecimal".to_string(),
            )
        })?;
    let configured_parent_hash = B256::from_str(&config.prev_hash).map_err(|_| {
        Error::CannotStartConsensusClient(
            "Configured EVM parent hash must be exactly 32 bytes of hexadecimal".to_string(),
        )
    })?;
    if configured_parent_hash != execution_anchor_hash {
        return Err(Error::CannotStartConsensusClient(format!(
            "Configured EVM parent hash {configured_parent_hash} does not match execution anchor hash {execution_anchor_hash}"
        )));
    }

    Checksum256::from_hex(&config.native_chain_id).map_err(|error| {
        Error::CannotStartConsensusClient(format!(
            "Native chain id must be exactly 32 bytes of hexadecimal: {error}"
        ))
    })?;
    Checksum256::from_hex(&config.execution_anchor_native_block_hash).map_err(|error| {
        Error::CannotStartConsensusClient(format!(
            "Native anchor hash must be exactly 32 bytes of hexadecimal: {error}"
        ))
    })?;
    let expected_native_parent = config
        .evm_start_block
        .checked_add(config.chain_id.block_delta())
        .and_then(|first_native_block| first_native_block.checked_sub(1))
        .ok_or_else(|| {
            Error::CannotStartConsensusClient(
                "EVM start block and native block delta do not have a valid parent".to_string(),
            )
        })?;
    if config.execution_anchor_native_block_number != expected_native_parent {
        return Err(Error::CannotStartConsensusClient(format!(
            "Native anchor block {} must be the parent {} of the first translated native block",
            config.execution_anchor_native_block_number, expected_native_parent
        )));
    }

    let mut client = ConsensusClient::new(args, config).await.map_err(|e| {
        warn!("Consensus client creation failed: {}", e);
        CannotStartConsensusClient(e.to_string())
    })?;

    info!(
        "Created client with latest EVM block: {:?}",
        client.latest_evm_number()
    );

    let lib = client.db.get_lib()?;

    if let Some(lib_number) = lib.as_ref().map(|lib| lib.number) {
        info!("Last stored LIB: {lib_number}");
    }

    if let Some(lib) = lib.as_ref() {
        let removed = client.db.prune_execution_branches(lib.number, &lib.hash)?;
        if removed > 0 {
            info!(
                removed,
                lib_number = lib.number,
                "Pruned irreversible branch entries"
            );
        }
    }

    let latest_number = lib
        .as_ref()
        .map(|lib| lib.number + client.config.chain_id.block_delta())
        .zip(client.latest_evm_number())
        .map(|(lib, latest)| cmp::min(lib, latest));

    let last_checked = match latest_number {
        Some(latest_number) => client.db.get_block_or_prev(latest_number)?,
        None => None,
    };

    if let Some(last_checked) = last_checked.as_ref() {
        info!(
            "Last stored final block: {}, {}",
            last_checked.number, last_checked.hash
        );
    }

    let branch_entries = client.db.get_execution_branch_entries()?;
    let stored_checkpoint = client.db.get_execution_checkpoint()?;
    let stored_checkpoint_is_canonical = if let Some(checkpoint) = stored_checkpoint.as_ref() {
        client
            .canonical_block_hash(checkpoint.branch.evm_block_number)
            .await?
            == Some(checkpoint.branch.evm_hash)
    } else {
        false
    };
    let latest_canonical = client
        .latest_executor_block
        .as_ref()
        .map(|block| -> Result<_, Error> {
            let number = u32::try_from(block.header.number).map_err(|_| {
                Error::CannotStartConsensusClient(format!(
                    "latest execution block {} exceeds the companion's u32 block range",
                    block.header.number
                ))
            })?;
            Ok((number, block.header.hash))
        })
        .transpose()?;
    let checkpoint = select_restart_checkpoint(
        stored_checkpoint,
        latest_canonical,
        &branch_entries,
        stored_checkpoint_is_canonical,
    )?;

    client.config.execution_branch_entries = branch_entries.clone();
    if let Some(checkpoint) = checkpoint {
        if client.db.get_execution_checkpoint()?.as_ref() != Some(&checkpoint) {
            client.db.put_execution_checkpoint(checkpoint.clone())?;
            info!(
                evm_block = checkpoint.branch.evm_block_number,
                evm_hash = %checkpoint.branch.evm_hash,
                "Recovered canonical checkpoint from execution forkchoice head"
            );
        }

        let resume_block = checkpoint.branch.evm_block_number.saturating_add(1);
        if client.is_in_start_stop_range(resume_block) {
            client.config.evm_start_block = resume_block;
            client.config.prev_hash = checkpoint.branch.evm_hash.to_string();
            client.config.validate_hash.clear();
            client.config.execution_context_anchor_block = resume_block;
            client.config.execution_context_starting_gas_price =
                checkpoint.branch.child_context.gas_price.to_string();
            client.config.execution_context_starting_revision =
                checkpoint.branch.child_context.revision;
            client.config.execution_context_parent_native_hash =
                Some(checkpoint.branch.native_hash);
            client.config.execution_context_parent_native_block =
                Some(checkpoint.branch.native_block_number);
        }
    } else if let Some(last_checked) = last_checked {
        if client.is_in_start_stop_range(last_checked.number + 1)
            && last_checked.number >= client.config.evm_start_block
            && latest_canonical.is_some_and(|(number, _)| number >= client.config.evm_start_block)
        {
            return Err(Error::CannotStartConsensusClient(format!(
                "Database has block {} but no accepted execution-context checkpoint; restart with --clean or supply a verified checkpoint",
                last_checked.number
            )));
        }
    }

    if let Some(sync_range) = client.sync_range() {
        if sync_range > client.config.maximum_sync_range {
            return Err(Error::RangeAboveMaximum(sync_range));
        }
    }
    Ok(client)
}

fn select_restart_checkpoint(
    stored_checkpoint: Option<ExecutionCheckpoint>,
    latest_canonical: Option<(u32, B256)>,
    branch_entries: &[ExecutionBranchEntry],
    stored_checkpoint_is_canonical: bool,
) -> Result<Option<ExecutionCheckpoint>, Error> {
    let canonical_branch = latest_canonical.and_then(|(number, hash)| {
        branch_entries
            .iter()
            .find(|entry| entry.evm_block_number == number && entry.evm_hash == hash)
            .cloned()
    });

    if let Some(canonical_branch) = canonical_branch {
        return Ok(Some(canonical_branch.into()));
    }
    if stored_checkpoint_is_canonical {
        return Ok(stored_checkpoint);
    }
    if let Some(checkpoint) = stored_checkpoint {
        return Err(Error::CannotStartConsensusClient(format!(
            "stored canonical checkpoint {} ({}) is not canonical in the execution endpoint and no durable branch entry matches its current head",
            checkpoint.branch.evm_block_number, checkpoint.branch.evm_hash
        )));
    }
    Ok(None)
}

fn parse_expected_first_child_hash(hash: &str) -> Result<B256, Error> {
    let hash = B256::from_str(hash).map_err(|_| {
        Error::CannotStartConsensusClient(
            "Expected first translated EVM block hash must be exactly 32 bytes of hexadecimal"
                .to_string(),
        )
    })?;
    if hash == B256::ZERO {
        return Err(Error::CannotStartConsensusClient(
            "Expected first translated EVM block hash cannot be zero".to_string(),
        ));
    }
    Ok(hash)
}

pub fn parse_log_level(s: &str) -> eyre::Result<LevelFilter> {
    match s.to_lowercase().as_str() {
        "off" => Ok(LevelFilter::OFF),
        "error" => Ok(LevelFilter::ERROR),
        "warn" => Ok(LevelFilter::WARN),
        "info" => Ok(LevelFilter::INFO),
        "debug" => Ok(LevelFilter::DEBUG),
        "trace" => Ok(LevelFilter::TRACE),
        _ => Err(eyre!("Unknown log level: {s}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;
    use telos_translator_rs::types::execution_metadata::TelosExecutionContext;

    fn branch(
        native_number: u32,
        native_hash: &str,
        evm_number: u32,
        byte: u8,
    ) -> ExecutionBranchEntry {
        ExecutionBranchEntry {
            native_block_number: native_number,
            native_hash: native_hash.to_string(),
            native_parent_hash: Some("parent".to_string()),
            evm_block_number: evm_number,
            evm_hash: B256::repeat_byte(byte),
            execution_base_fee: U256::from(7),
            child_context: TelosExecutionContext {
                gas_price: U256::from(byte),
                revision: u64::from(byte),
            },
        }
    }

    #[test]
    fn crash_before_fcu_does_not_promote_last_valid_side_branch() {
        let canonical = ExecutionCheckpoint::from(branch(136, "canonical", 100, 1));
        let side = branch(137, "valid-side", 101, 2);

        let selected = select_restart_checkpoint(
            Some(canonical.clone()),
            Some((100, canonical.branch.evm_hash)),
            &[side],
            true,
        )
        .unwrap()
        .unwrap();

        assert_eq!(selected, canonical);
    }

    #[test]
    fn restart_after_reorg_selects_branch_matching_execution_head() {
        let old = ExecutionCheckpoint::from(branch(236, "old", 200, 3));
        let reorged = branch(237, "reorged", 201, 4);

        let selected = select_restart_checkpoint(
            Some(old),
            Some((reorged.evm_block_number, reorged.evm_hash)),
            std::slice::from_ref(&reorged),
            false,
        )
        .unwrap()
        .unwrap();

        assert_eq!(selected.branch, reorged);
    }

    #[test]
    fn noncanonical_checkpoint_without_a_matching_head_fails_closed() {
        let side = ExecutionCheckpoint::from(branch(336, "side", 300, 5));
        let result =
            select_restart_checkpoint(Some(side), Some((300, B256::repeat_byte(6))), &[], false);

        assert!(result.is_err());
    }

    #[test]
    fn first_child_hash_must_be_nonzero_and_well_formed() {
        let expected = B256::repeat_byte(0x77);
        assert_eq!(
            parse_expected_first_child_hash(&expected.to_string()).unwrap(),
            expected
        );
        assert!(parse_expected_first_child_hash(&B256::ZERO.to_string()).is_err());
        assert!(parse_expected_first_child_hash("not-a-hash").is_err());
    }
}
