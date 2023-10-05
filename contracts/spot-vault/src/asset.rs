use std::collections::{HashMap, HashSet};
use std::fmt;

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    coin, Addr, Api, BankMsg, Coin, CosmosMsg, CustomMsg, StdError, StdResult, Uint128,
};

use itertools::Itertools;

/// Maximum denom length
pub const DENOM_MAX_LENGTH: usize = 60;

/// This enum describes a CW20 asset.
#[cw_serde]
pub struct Asset {
    /// Information about an asset stored in a [`AssetInfo`] struct
    pub info: AssetInfo,
    /// A token amount
    pub amount: Uint128,
}

impl fmt::Display for Asset {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}{}", self.amount, self.info)
    }
}

impl Asset {
    /// For native tokens of type [`AssetInfo`] uses the default method [`BankMsg::Send`] to send a
    /// token amount to a recipient.
    pub fn into_msg<T>(self, recipient: impl Into<String>) -> StdResult<CosmosMsg<T>>
    where
        T: CustomMsg,
    {
        let recipient = recipient.into();
        Ok(CosmosMsg::Bank(BankMsg::Send {
            to_address: recipient,
            amount: vec![self.as_coin()?],
        }))
    }

    pub fn as_coin(&self) -> StdResult<Coin> {
        Ok(coin(self.amount.u128(), &self.info.denom))
    }
}

pub trait CoinsExt {
    fn assert_coins_properly_sent(
        &self,
        assets: &[Asset],
        pool_asset_infos: &[AssetInfo],
    ) -> StdResult<()>;
}

impl CoinsExt for Vec<Coin> {
    fn assert_coins_properly_sent(
        &self,
        input_assets: &[Asset],
        pool_asset_infos: &[AssetInfo],
    ) -> StdResult<()> {
        let pool_coins = pool_asset_infos
            .iter()
            .filter_map(|asset_info| Some(asset_info.denom.to_string()))
            .collect::<HashSet<_>>();

        let input_coins = input_assets
            .iter()
            .filter_map(|asset| Some((asset.info.denom.to_string(), asset.amount)))
            .map(|pair| {
                if pool_coins.contains(&pair.0) {
                    Ok(pair)
                } else {
                    Err(StdError::generic_err(format!(
                        "Asset {} is not in the pool",
                        pair.0
                    )))
                }
            })
            .collect::<StdResult<HashMap<_, _>>>()?;

        self.iter().try_for_each(|coin| {
            if input_coins.contains_key(&coin.denom) {
                if input_coins[&coin.denom] == coin.amount {
                    Ok(())
                } else {
                    Err(StdError::generic_err(
                        "Native token balance mismatch between the argument and the transferred",
                    ))
                }
            } else {
                Err(StdError::generic_err(format!(
                    "Supplied coins contain {} that is not in the input asset vector",
                    coin.denom
                )))
            }
        })
    }
}

#[cw_serde]
pub struct AssetInfo {
    pub denom: String,
}

impl fmt::Display for AssetInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.denom)
    }
}

impl AssetInfo {
    /// Returns **true** if the calling token is the same as the token specified in the input parameters.
    /// Otherwise returns **false**.
    pub fn equal(&self, another_asset: &AssetInfo) -> bool {
        self.denom == another_asset.denom
    }

    /// Checks that the tokens' denom or contract addr is valid.
    pub fn check(&self, _api: &dyn Api) -> StdResult<()> {
        let denom = &self.denom;
        if !is_valid_symbol(denom, Some(DENOM_MAX_LENGTH)) {
            return Err(StdError::generic_err(format!(
                "Native denom is not in expected format [a-zA-Z\\-][3,{DENOM_MAX_LENGTH}]: {denom}",
            )));
        }
        Ok(())
    }
}

/// Returns a lowercased, validated address upon success if present.
#[inline]
pub fn addr_opt_validate(api: &dyn Api, addr: &Option<String>) -> StdResult<Option<Addr>> {
    addr.as_ref()
        .map(|addr| api.addr_validate(addr))
        .transpose()
}

const TOKEN_SYMBOL_MAX_LENGTH: usize = 4;

/// Returns a formatted LP token name
pub fn format_lp_token_name(denom0: &String, denom1: &String) -> StdResult<String> {
    let mut short_denoms: Vec<String> = vec![];
    let short_denom0 = denom0.chars().take(TOKEN_SYMBOL_MAX_LENGTH).collect();
    short_denoms.push(short_denom0);
    let short_denom1 = denom1.chars().take(TOKEN_SYMBOL_MAX_LENGTH).collect();
    short_denoms.push(short_denom1);
    Ok(format!("{}-LP", short_denoms.iter().join("-")).to_uppercase())
}

/// Checks the validity of the token symbol
fn is_valid_symbol(symbol: &str, max_length: Option<usize>) -> bool {
    let max_length = max_length.unwrap_or(12);
    let bytes = symbol.as_bytes();
    if bytes.len() < 3 || bytes.len() > max_length {
        return false;
    }
    for byte in bytes.iter() {
        if (*byte != 45)
            && (*byte < 47 || *byte > 57)
            && (*byte < 65 || *byte > 90)
            && (*byte < 97 || *byte > 122)
        {
            return false;
        }
    }
    true
}
