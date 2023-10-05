use cosmwasm_std::{CustomQuery, QuerierWrapper, StdResult, Uint128};
use cw20::{BalanceResponse as Cw20BalanceResponse, Cw20QueryMsg, TokenInfoResponse};

/// Returns a native token's balance for a specific account.
///
/// * **denom** specifies the denomination used to return the balance (e.g uluna).
pub fn query_balance<C>(
    querier: &QuerierWrapper<C>,
    account_addr: impl Into<String>,
    denom: impl Into<String>,
) -> StdResult<Uint128>
where
    C: CustomQuery,
{
    querier
        .query_balance(account_addr, denom)
        .map(|coin| coin.amount)
}

/// Returns a token balance for an account.
///
/// * **contract_addr** token contract for which we return a balance.
///
/// * **account_addr** account address for which we return a balance.
pub fn query_token_balance<C>(
    querier: &QuerierWrapper<C>,
    contract_addr: impl Into<String>,
    account_addr: impl Into<String>,
) -> StdResult<Uint128>
where
    C: CustomQuery,
{
    // load balance from the token contract
    let resp: Cw20BalanceResponse = querier
        .query_wasm_smart(
            contract_addr,
            &Cw20QueryMsg::Balance {
                address: account_addr.into(),
            },
        )
        .unwrap_or_else(|_| Cw20BalanceResponse {
            balance: Uint128::zero(),
        });

    Ok(resp.balance)
}

/// Returns the total supply of a specific token.
///
/// * **contract_addr** token contract address.
pub fn query_supply<C>(
    querier: &QuerierWrapper<C>,
    contract_addr: impl Into<String>,
) -> StdResult<Uint128>
where
    C: CustomQuery,
{
    let res: TokenInfoResponse =
        querier.query_wasm_smart(contract_addr, &Cw20QueryMsg::TokenInfo {})?;

    Ok(res.total_supply)
}
