use cosmwasm_std::Uint128;
use cw20::Cw20ReceiveMsg;
use cw_ownable::Action;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use injective_cosmwasm::MarketId;
use injective_math::FPDecimal;

use crate::asset::Asset;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub owner: String,
    pub market_id: MarketId,
    pub quote_decimal: u8,
    pub hardcap: Uint128,
    pub token_code_id: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    /// Update Ownership
    UpdateOwnership(Action),
    Receive(Cw20ReceiveMsg),
    /// Deposit allows someone to deposit in the vault
    Deposit {
        /// The amounts to deposit
        assets: Vec<Asset>,
        /// The receiver of LP tokens
        receiver: Option<String>,
    },
    /// SwapPerpetual
    SwapPerpetual {
        long: bool,
        quantity: FPDecimal,
        price: FPDecimal,
        margin: FPDecimal,
    },
    /// Cancel placed order
    CancelOrder {
        order_hash: String,
    },
    /// Add fee
    AddFee {
        fee: Uint128,
    },
    /// Withdraw fee
    WithdrawFee {
        fee: Uint128,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Ownership {},
    TokensForShares { share: Uint128 },
    TotalLiquidity {},
    UserLiquidity { user: String },
    Tokens {},
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Cw20HookMsg {
    Withdraw {},
}
