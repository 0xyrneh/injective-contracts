use cosmwasm_std::{Addr, Uint128};
use cw_storage_plus::Item;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use injective_cosmwasm::{MarketId, SubaccountId};

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
pub struct ContractInfo {
    pub market_id: MarketId,
    pub base_denom: String,
    pub quote_denom: String,
    pub base_decimal: u8,
    pub quote_decimal: u8,
    pub base_price_id: String,
    pub quote_price_id: String,
    pub hardcap: Uint128,
    pub liquidity_token: Addr,
    pub contract_subaccount_id: SubaccountId,
}

pub const CONTRACT_INFO: Item<ContractInfo> = Item::new("vault");

pub const BASE_FEE_COLLECTED: Item<Uint128> = Item::new("base_fee_collected");

pub const QUOTE_FEE_COLLECTED: Item<Uint128> = Item::new("quote_fee_collected");
