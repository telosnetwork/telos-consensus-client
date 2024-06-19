use alloy::primitives::B256;
use antelope::api::client::{APIClient, Provider};
use antelope::chain::asset::{Asset, Symbol};
use antelope::chain::checksum::Checksum256;
use antelope::chain::name::Name;
use antelope::util::hex_to_bytes;
use telos_translator_rs::transaction::Transaction;
use telos_translator_rs::types::evm_types::TransferAction;
use telos_translator_rs::types::types::NameToAddressCache;

#[derive(Debug, Default, Clone)]
struct MockProvider;

#[async_trait::async_trait]
impl Provider for MockProvider {
    async fn post(&self, path: String, body: Option<String>) -> Result<String, String> {
        // TODO: Mock this for tests
        return Ok("".to_string());
    }

    async fn get(&self, path: String) -> Result<String, String> {
        todo!()
    }
}

#[tokio::test]
async fn test_deposit() {
    let trx = Transaction::from_transfer(
        40,
        0,
        Checksum256::from_bytes(&hex_to_bytes("11e1a6c5c637681588383e401479054882c2a168f1ea766bedcc75a9ca4ce6b8")).unwrap(),
        TransferAction {
            from: Name::new("exrsrv.tf"),
            to: Name::new("eosio.evm"),
            quantity: Asset::new(654507, Symbol::new("TLOS", 4)),
            memo: "0xb4b01216a5bc8f1c8a33cd990a1239030e60c905".to_string(),
        }, &NameToAddressCache::new(APIClient::default())).await;

    assert_eq!(trx.hash().to_string(), "0xc92303ea310408950f009ac1466d4c4d534cfd02a4bc630881e94e28da7b377e");
}