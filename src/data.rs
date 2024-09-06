use std::{fs, path::Path, sync::Arc};

use eyre::{eyre, Context};
use rocksdb::{DBWithThreadMode, SingleThreaded, DB};
use serde::{Deserialize, Serialize};
use telos_translator_rs::block::TelosEVMBlock;
use tracing::info;

use crate::client::Error;

#[derive(Clone, Serialize, Deserialize)]
pub struct Block {
    pub number: u32,
    pub hash: String,
}

pub struct Lib<'a>(pub &'a TelosEVMBlock);

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

pub struct Database {
    db: Arc<DBWithThreadMode<SingleThreaded>>,
}

impl Database {
    fn block_key(number: u32) -> String {
        format!("blocks:{number}")
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

    pub fn put_lib(&self, lib: Block) -> Result<(), Error> {
        let value = serde_json::to_string(&lib)
            .wrap_err("Failed to serialize lib")
            .map_err(Error::Database)?;

        self.db
            .put("lib", value)
            .wrap_err("Failed to put lib into database")
            .map_err(Error::Database)
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
