use antelope::chain::checksum::Checksum256;
use antelope::chain::name::Name;
use antelope::chain::public_key::PublicKey;
use antelope::chain::signature::Signature;
use antelope::chain::varint::VarUint32;
use antelope::chain::{Decoder, Encoder};
use antelope::serializer::packer::Float128;
use antelope::serializer::Packer;
use antelope::{EnumPacker, StructPacker};
use serde::{Deserialize, Serialize};
use std::option::Option;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Clone, Serialize, Deserialize, EnumPacker)]
pub enum ShipRequest {
    GetStatus(GetStatusRequestV0),
    GetBlocks(GetBlocksRequestV0),
    GetBlocksAck(GetBlocksAckRequestV0),
}

impl From<&ShipRequest> for Message {
    fn from(value: &ShipRequest) -> Self {
        Message::Binary(Encoder::pack(value))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, EnumPacker)]
pub enum ShipResult {
    GetStatusResultV0(GetStatusResultV0),
    GetBlocksResultV0(GetBlocksResultV0),
}

#[derive(Debug, Clone, Serialize, Deserialize, EnumPacker)]
pub enum TransactionTrace {
    V0(TransactionTraceV0),
}

#[derive(Debug, Clone, Serialize, Deserialize, EnumPacker)]
pub enum PartialTransaction {
    V0(PartialTransactionV0),
}

#[derive(Debug, Clone, Serialize, Deserialize, EnumPacker)]
pub enum ActionTrace {
    V0(ActionTraceV0),
    V1(ActionTraceV1),
}

#[derive(Debug, Clone, Serialize, Deserialize, EnumPacker)]
pub enum TableDelta {
    V0(TableDeltaV0),
}

#[derive(Debug, Clone, Serialize, Deserialize, EnumPacker)]
pub enum ActionReceipt {
    V0(ActionReceiptV0),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GetStatusRequestV0;

impl Packer for GetStatusRequestV0 {
    fn size(&self) -> usize {
        0
    }

    fn pack(&self, _enc: &mut Encoder) -> usize {
        0
    }

    fn unpack(&mut self, _data: &[u8]) -> usize {
        0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct BlockPosition {
    pub block_num: u32,
    pub block_id: Checksum256,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct GetBlocksRequestV0 {
    pub start_block_num: u32,
    pub end_block_num: u32,
    pub max_messages_in_flight: u32,
    pub have_positions: Vec<BlockPosition>,
    pub irreversible_only: bool,
    pub fetch_block: bool,
    pub fetch_traces: bool,
    pub fetch_deltas: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct GetBlocksAckRequestV0 {
    pub num_messages: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct GetStatusResultV0 {
    pub head: BlockPosition,
    pub last_irreversible: BlockPosition,
    pub trace_begin_block: u32,
    pub trace_end_block: u32,
    pub chain_state_begin_block: u32,
    pub chain_state_end_block: u32,
    pub chain_id: Checksum256,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct GetBlocksResultV0 {
    pub head: BlockPosition,
    pub last_irreversible: BlockPosition,
    pub this_block: Option<BlockPosition>,
    pub prev_block: Option<BlockPosition>,
    pub block: Option<Vec<u8>>,
    pub traces: Option<Vec<u8>>,
    pub deltas: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct Row {
    pub present: bool,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct TableDeltaV0 {
    pub name: String,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct Action {
    pub account: Name,
    pub name: Name,
    pub authorization: Vec<PermissionLevel>,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct AccountAuthSequence {
    pub account: Name,
    pub sequence: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ActionReceiptV0 {
    pub receiver: Name,
    pub act_digest: Checksum256,
    pub global_sequence: u64,
    pub recv_sequence: u64,
    pub auth_sequence: Vec<AccountAuthSequence>,
    pub code_sequence: VarUint32,
    pub abi_sequence: VarUint32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct AccountDelta {
    pub account: Name,
    pub delta: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ActionTraceV0 {
    pub action_ordinal: VarUint32,
    pub creator_action_ordinal: VarUint32,
    pub receipt: Option<ActionReceipt>,
    pub receiver: Name,
    pub act: Action,
    pub context_free: bool,
    pub elapsed: i64,
    pub console: String,
    pub account_ram_deltas: Vec<AccountDelta>,
    pub except: Option<String>,
    pub error_code: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ActionTraceV1 {
    pub action_ordinal: VarUint32,
    pub creator_action_ordinal: VarUint32,
    pub receipt: Option<ActionReceipt>,
    pub receiver: Name,
    pub act: Action,
    pub context_free: bool,
    pub elapsed: i64,
    pub console: String,
    pub account_ram_deltas: Vec<AccountDelta>,
    pub except: Option<String>,
    pub error_code: Option<u64>,
    pub return_value: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct PartialTransactionV0 {
    pub expiration: u32,
    pub ref_block_num: u16,
    pub ref_block_prefix: u32,
    pub max_net_usage_words: VarUint32,
    pub max_cpu_usage_ms: u8,
    pub delay_sec: VarUint32,
    pub transaction_extensions: Vec<Extension>,
    pub signatures: Vec<Signature>,
    pub context_free_data: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct TransactionTraceV0 {
    pub id: Checksum256,
    pub status: u8,
    pub cpu_usage_us: u32,
    pub net_usage_words: VarUint32,
    pub elapsed: i64,
    pub net_usage: u64,
    pub scheduled: bool,
    pub action_traces: Vec<ActionTrace>,
    pub account_ram_delta: Option<AccountDelta>,
    pub except: Option<String>,
    pub error_code: Option<u64>,
    pub failed_dtrx_trace: Option<Box<TransactionTrace>>,
    pub partial: Option<PartialTransaction>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct PackedTransaction {
    pub signatures: Vec<Signature>,
    pub compression: u8,
    pub packed_context_free_data: Vec<u8>,
    pub packed_trx: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, EnumPacker)]
pub enum TransactionVariant {
    ID(Checksum256),
    Packed(PackedTransaction),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct TransactionReceipt {
    pub status: u8,
    pub cpu_usage_us: u32,
    pub net_usage_words: VarUint32,
    pub trx: TransactionVariant,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct Extension {
    pub r#type: u16,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct BlockHeader {
    pub timestamp: u32,
    pub producer: Name,
    pub confirmed: u16,
    pub previous: Checksum256,
    pub transaction_mroot: Checksum256,
    pub action_mroot: Checksum256,
    pub schedule_version: u32,
    pub new_producers: Option<Vec<u8>>,
    pub header_extensions: Vec<Extension>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct SignedBlockHeader {
    pub header: BlockHeader,
    pub producer_signature: Signature,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct SignedBlock {
    pub header: SignedBlockHeader,
    pub transactions: Vec<TransactionReceipt>,
    pub block_extensions: Vec<Extension>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct CodeId {
    pub vm_type: u8,
    pub vm_version: u8,
    pub code_hash: Checksum256,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct AccountV0 {
    pub name: Name,
    pub creation_date: u32,
    pub abi: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct AccountMetadataV0 {
    pub name: Name,
    pub privileged: bool,
    pub last_code_update: u64,
    pub code: Option<CodeId>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct CodeV0 {
    pub vm_type: u8,
    pub vm_version: u8,
    pub code_hash: Checksum256,
    pub code: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ContractTableV0 {
    pub code: Name,
    pub scope: Name,
    pub table: Name,
    pub payer: Name,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ContractRowV0 {
    pub code: Name,
    pub scope: Name,
    pub table: Name,
    pub primary_key: u64,
    pub payer: Name,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ContractIndex64V0 {
    pub code: Name,
    pub scope: Name,
    pub table: Name,
    pub primary_key: u64,
    pub payer: Name,
    pub secondary_key: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ContractIndex128V0 {
    pub code: Name,
    pub scope: Name,
    pub table: Name,
    pub primary_key: u64,
    pub payer: Name,
    pub secondary_key: u128,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ContractIndex256V0 {
    pub code: Name,
    pub scope: Name,
    pub table: Name,
    pub primary_key: u64,
    pub payer: Name,
    pub secondary_key: Checksum256,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ContractIndexDoubleV0 {
    pub code: Name,
    pub scope: Name,
    pub table: Name,
    pub primary_key: u64,
    pub payer: Name,
    pub secondary_key: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ContractIndexLongDoubleV0 {
    pub code: Name,
    pub scope: Name,
    pub table: Name,
    pub primary_key: u64,
    pub payer: Name,
    pub secondary_key: Float128,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ProducerKey {
    pub producer_name: Name,
    pub block_signing_key: PublicKey,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ProducerSchedule {
    pub version: u32,
    pub producers: Vec<ProducerKey>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct BlockSigningAuthorityV0 {
    pub threshold: u32,
    pub keys: Vec<KeyWeight>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ProducerAuthority {
    pub producer_name: Name,
    pub authority: BlockSigningAuthorityV0,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ProducerAuthoritySchedule {
    pub version: u32,
    pub producers: Vec<ProducerAuthority>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ChainConfigV0 {
    pub max_block_net_usage: u64,
    pub target_block_net_usage_pct: u32,
    pub max_transaction_net_usage: u32,
    pub base_per_transaction_net_usage: u32,
    pub net_usage_leeway: u32,
    pub context_free_discount_net_usage_num: u32,
    pub context_free_discount_net_usage_den: u32,
    pub max_block_cpu_usage: u32,
    pub target_block_cpu_usage_pct: u32,
    pub max_transaction_cpu_usage: u32,
    pub min_transaction_cpu_usage: u32,
    pub max_transaction_lifetime: u32,
    pub deferred_trx_expiration_window: u32,
    pub max_transaction_delay: u32,
    pub max_inline_action_size: u32,
    pub max_inline_action_depth: u16,
    pub max_authority_depth: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ChainConfigV1 {
    pub max_block_net_usage: u64,
    pub target_block_net_usage_pct: u32,
    pub max_transaction_net_usage: u32,
    pub base_per_transaction_net_usage: u32,
    pub net_usage_leeway: u32,
    pub context_free_discount_net_usage_num: u32,
    pub context_free_discount_net_usage_den: u32,
    pub max_block_cpu_usage: u32,
    pub target_block_cpu_usage_pct: u32,
    pub max_transaction_cpu_usage: u32,
    pub min_transaction_cpu_usage: u32,
    pub max_transaction_lifetime: u32,
    pub deferred_trx_expiration_window: u32,
    pub max_transaction_delay: u32,
    pub max_inline_action_size: u32,
    pub max_inline_action_depth: u16,
    pub max_authority_depth: u16,
    pub max_action_return_value_size: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct WasmConfigV0 {
    pub max_mutable_global_bytes: u32,
    pub max_table_elements: u32,
    pub max_section_elements: u32,
    pub max_linear_memory_init: u32,
    pub max_func_local_bytes: u32,
    pub max_nested_structures: u32,
    pub max_symbol_bytes: u32,
    pub max_module_bytes: u32,
    pub max_code_bytes: u32,
    pub max_pages: u32,
    pub max_call_depth: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct GlobalPropertyV0 {
    pub proposed_schedule_block_num: Option<u32>,
    pub proposed_schedule: ProducerSchedule,
    pub configuration: ChainConfigV0,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct GlobalPropertyV1 {
    pub proposed_schedule_block_num: Option<u32>,
    pub proposed_schedule: ProducerAuthoritySchedule,
    pub configuration: ChainConfigV0,
    pub chain_id: Checksum256,
    pub wasm_configuration: Option<WasmConfigV0>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct GeneratedTransactionV0 {
    pub sender: Name,
    pub sender_id: u128,
    pub payer: Name,
    pub trx_id: Checksum256,
    pub packed_trx: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ActivatedProtocolFeatureV0 {
    pub feature_digest: Checksum256,
    pub activation_block_num: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ProtocolStateV0 {
    pub activated_protocol_features: Vec<ActivatedProtocolFeatureV0>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct KeyWeight {
    pub key: PublicKey,
    pub weight: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct PermissionLevel {
    pub actor: Name,
    pub permission: Name,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct PermissionLevelWeight {
    pub permission: PermissionLevel,
    pub weight: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct WaitWeight {
    pub wait_sec: u32,
    pub weight: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct Authority {
    pub threshold: u32,
    pub keys: Vec<KeyWeight>,
    pub accounts: Vec<PermissionLevelWeight>,
    pub waits: Vec<WaitWeight>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct PermissionV0 {
    pub owner: Name,
    pub name: Name,
    pub parent: Name,
    pub last_updated: u64,
    pub auth: Authority,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct PermissionLinkV0 {
    pub account: Name,
    pub code: Name,
    pub message_type: Name,
    pub required_permission: Name,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ResourceLimitsV0 {
    pub owner: Name,
    pub net_weight: i64,
    pub cpu_weight: i64,
    pub ram_bytes: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct UsageAccumulatorV0 {
    pub last_ordinal: u32,
    pub value_ex: u64,
    pub consumed: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ResourceUsageV0 {
    pub owner: Name,
    pub net_usage: UsageAccumulatorV0,
    pub cpu_usage: UsageAccumulatorV0,
    pub ram_usage: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ResourceLimitsStateV0 {
    pub average_block_net_usage: UsageAccumulatorV0,
    pub average_block_cpu_usage: UsageAccumulatorV0,
    pub total_net_weight: u64,
    pub total_cpu_weight: u64,
    pub total_ram_bytes: u64,
    pub virtual_net_limit: u64,
    pub virtual_cpu_limit: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ResourceLimitsRatioV0 {
    pub numerator: u64,
    pub denominator: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ElasticLimitParametersV0 {
    pub target: u64,
    pub max: u64,
    pub periods: u32,
    pub max_multiplier: u32,
    pub contract_rate: ResourceLimitsRatioV0,
    pub expand_rate: ResourceLimitsRatioV0,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ResourceLimitsConfigV0 {
    pub cpu_limit_parameters: ElasticLimitParametersV0,
    pub net_limit_parameters: ElasticLimitParametersV0,
    pub account_cpu_usage_average_window: u32,
    pub account_net_usage_average_window: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct BlockSigningAuthority {
    pub threshold: u32,
    pub keys: Vec<KeyWeight>,
}
