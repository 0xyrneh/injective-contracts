use cosmwasm_std::entry_point;
use cosmwasm_std::{
    attr, from_binary, to_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env,
    MessageInfo, Reply, ReplyOn, Response, StdError, StdResult, Storage, SubMsg, Uint128, WasmMsg,
};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg, MinterResponse};
use cw20_base::msg::InstantiateMsg as TokenInstantiateMsg;
use cw_ownable::{get_ownership, is_owner, update_ownership};
use injective_math::scale::Scaled;
use injective_math::FPDecimal;
use injective_protobuf::proto::tx;
use protobuf::Message;
#[cfg(not(feature = "library"))]
use std::cmp::min;

use injective_cosmwasm::{
    cancel_spot_order_msg, create_batch_update_orders_msg,
    get_default_subaccount_id_for_checked_address, InjectiveMsgWrapper, InjectiveQuerier,
    InjectiveQueryWrapper, MarketStatus, OrderType, SpotOrder,
};

use crate::asset::{addr_opt_validate, format_lp_token_name, Asset, AssetInfo, CoinsExt};
use crate::error::ContractError;
use crate::msg::{Cw20HookMsg, ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::querier::{query_balance, query_supply, query_token_balance};
use crate::response::MsgInstantiateContractResponse;
use crate::state::{ContractInfo, BASE_FEE_COLLECTED, CONTRACT_INFO, QUOTE_FEE_COLLECTED};

/// A `reply` call code ID used for sub-messages.
pub const INSTANTIATE_TOKEN_REPLY_ID: u64 = 1u64;
pub const ORDER_REPLY_ID: u64 = 2u64;
pub const PRICE_VALID_DURATION: i64 = 60; // 1 min

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut<InjectiveQueryWrapper>,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    let querier = InjectiveQuerier::new(&deps.querier);
    if let Some(market) = querier.query_spot_market(&msg.market_id)?.market {
        if market.status != MarketStatus::Active {
            return Err(ContractError::CustomError {
                val: format!("Market with id: {} not active", msg.market_id.as_str()),
            });
        }
        cw_ownable::initialize_owner(deps.storage, deps.api, Some(msg.owner.as_str()))
            .expect(format!("Invalid owner: {}", msg.owner).as_str());
        let contract_info = ContractInfo {
            market_id: msg.market_id,
            base_denom: market.base_denom,
            quote_denom: market.quote_denom,
            base_decimal: msg.base_decimal,
            quote_decimal: msg.quote_decimal,
            base_price_id: msg.base_price_id,
            quote_price_id: msg.quote_price_id,
            hardcap: msg.hardcap,
            liquidity_token: Addr::unchecked(""),
            contract_subaccount_id: get_default_subaccount_id_for_checked_address(
                &env.contract.address,
            ),
        };
        CONTRACT_INFO.save(deps.storage, &contract_info)?;
        BASE_FEE_COLLECTED.save(deps.storage, &Uint128::zero())?;
        QUOTE_FEE_COLLECTED.save(deps.storage, &Uint128::zero())?;
        let token_name =
            format_lp_token_name(&contract_info.base_denom, &contract_info.quote_denom)?;

        // Create the LP token contract
        let sub_msg: Vec<SubMsg<InjectiveMsgWrapper>> = vec![SubMsg {
            msg: WasmMsg::Instantiate {
                code_id: msg.token_code_id,
                msg: to_binary(&TokenInstantiateMsg {
                    name: token_name,
                    symbol: "uLP".to_string(),
                    decimals: 12,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: env.contract.address.to_string(),
                        cap: None,
                    }),
                    marketing: None,
                })?,
                funds: vec![],
                admin: None,
                label: String::from("Elixir LP token"),
            }
            .into(),
            id: INSTANTIATE_TOKEN_REPLY_ID,
            gas_limit: None,
            reply_on: ReplyOn::Success,
        }];

        Ok(Response::<InjectiveMsgWrapper>::new()
            .add_submessages(sub_msg)
            .add_attribute("method", "instantiate"))
    } else {
        Err(ContractError::CustomError {
            val: format!("Market with id: {} not found", msg.market_id.as_str()),
        })
    }
}

/// The entry point to the contract for processing replies from submessages.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(
    deps: DepsMut<InjectiveQueryWrapper>,
    env: Env,
    msg: Reply,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    match msg.id {
        INSTANTIATE_TOKEN_REPLY_ID => handle_instantiate_token_reply(deps, env, msg),
        ORDER_REPLY_ID => handle_order_reply(deps, env, msg),
        _ => Err(ContractError::UnrecognisedReply(msg.id)),
    }
}

fn handle_instantiate_token_reply(
    deps: DepsMut<InjectiveQueryWrapper>,
    _env: Env,
    msg: Reply,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    let mut contract_info = CONTRACT_INFO.load(deps.storage)?;

    if contract_info.liquidity_token != Addr::unchecked("") {
        return Err(ContractError::Unauthorized {});
    }

    let data = msg
        .result
        .into_result()
        .expect("no result available")
        .data
        .expect("no data available");
    let res: MsgInstantiateContractResponse =
        Message::parse_from_bytes(data.as_slice()).map_err(|_| {
            StdError::parse_err("MsgInstantiateContractResponse", "failed to parse data")
        })?;

    contract_info.liquidity_token = deps.api.addr_validate(res.get_contract_address())?;

    CONTRACT_INFO.save(deps.storage, &contract_info)?;

    Ok(Response::<InjectiveMsgWrapper>::new()
        .add_attribute("liquidity_token_addr", contract_info.liquidity_token))
}

fn handle_order_reply(
    _deps: DepsMut<InjectiveQueryWrapper>,
    _env: Env,
    msg: Reply,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    let id = msg.id;
    let order_response: tx::MsgBatchUpdateOrdersResponse = Message::parse_from_bytes(
        msg.result
            .into_result()
            .map_err(ContractError::SubMsgFailure)?
            .data
            .ok_or_else(|| ContractError::ReplyParseFailure {
                id,
                err: "Missing reply data".to_owned(),
            })?
            .as_slice(),
    )
    .map_err(|err| ContractError::ReplyParseFailure {
        id,
        err: err.to_string(),
    })?;

    let order_hash = order_response.spot_order_hashes.into_vec()[0].clone();

    Ok(Response::new().add_attributes(vec![attr("order_hash", order_hash)]))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut<InjectiveQueryWrapper>,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    match msg {
        ExecuteMsg::UpdateOwnership(action) => {
            let res = update_ownership(deps.into_empty(), &env.block, &info.sender, action);
            match res {
                Ok(_) => {
                    return Ok(Response::new());
                }
                Err(err) => {
                    return Err(ContractError::CustomError {
                        val: format!("Update ownership failed with {}", err),
                    });
                }
            }
        }
        ExecuteMsg::Receive(msg) => receive_cw20(deps, env, info, msg),
        ExecuteMsg::Deposit { assets, receiver } => deposit(deps, env, info, assets, receiver),
        ExecuteMsg::SwapSpot {
            buying,
            quantity,
            price,
        } => try_swap(deps, env, info, buying, quantity, price),
        ExecuteMsg::CancelOrder { order_hash } => try_cancel_order(deps, env, info, order_hash),
        ExecuteMsg::AddFee {
            base_fee,
            quote_fee,
        } => add_fee(deps, env, info, base_fee, quote_fee),
        ExecuteMsg::WithdrawFee {
            base_fee,
            quote_fee,
        } => withdraw_fee(deps, env, info, base_fee, quote_fee),
    }
}

/// Receives a message of type [`Cw20ReceiveMsg`] and processes it depending on the received template.
///
/// * **cw20_msg** is the CW20 message that has to be processed.
fn receive_cw20(
    deps: DepsMut<InjectiveQueryWrapper>,
    env: Env,
    info: MessageInfo,
    cw20_msg: Cw20ReceiveMsg,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    match from_binary(&cw20_msg.msg) {
        Ok(Cw20HookMsg::Withdraw {}) => withdraw(
            deps,
            env,
            info,
            Addr::unchecked(cw20_msg.sender),
            cw20_msg.amount,
        ),
        Err(err) => Err(err.into()),
    }
}

/// Deposit tokens with the specified input parameters.
///
/// * **assets** is an array with assets supported by vault.
///
/// * **receiver** is an optional parameter which defines the receiver of the LP tokens.
/// If no custom receiver is specified, the vault will mint LP tokens for the function caller.
fn deposit(
    deps: DepsMut<InjectiveQueryWrapper>,
    env: Env,
    info: MessageInfo,
    assets: Vec<Asset>,
    receiver: Option<String>,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    if assets.len() != 2 {
        return Err(StdError::generic_err("assets must contain exactly two elements").into());
    }
    assets[0].info.check(deps.api)?;
    assets[1].info.check(deps.api)?;

    let contract_info = CONTRACT_INFO.load(deps.storage)?;

    let supported = vec![
        AssetInfo {
            denom: contract_info.base_denom.clone(),
        },
        AssetInfo {
            denom: contract_info.quote_denom.clone(),
        },
    ];
    info.funds.assert_coins_properly_sent(&assets, &supported)?;

    let amounts = [
        assets
            .iter()
            .find(|a| a.info.equal(&supported[0]))
            .map(|a| a.amount)
            .expect("Wrong asset info is given"),
        assets
            .iter()
            .find(|a| a.info.equal(&supported[1]))
            .map(|a| a.amount)
            .expect("Wrong asset info is given"),
    ];

    let prices = get_prices(deps.as_ref(), env.clone())?;

    let scaled_amount0 = FPDecimal::from(amounts[0]).scaled(-(contract_info.base_decimal as i32));
    let scaled_amount1 = FPDecimal::from(amounts[1]).scaled(-(contract_info.quote_decimal as i32));

    let token0_value = scaled_amount0 * prices[0];
    let token1_value = scaled_amount1 * prices[1];
    let single_deposit_value = min(token0_value, token1_value);

    let actual_deposits = [
        single_deposit_value / prices[0],
        single_deposit_value / prices[1],
    ];

    if actual_deposits[0].is_zero() || actual_deposits[1].is_zero() {
        return Err(ContractError::InvalidZeroAmount {});
    }

    let unscaled_amount0 = Uint128::new(u128::from(
        actual_deposits[0].scaled(contract_info.base_decimal as i32),
    ));
    let unscaled_amount1 = Uint128::new(u128::from(
        actual_deposits[1].scaled(contract_info.quote_decimal as i32),
    ));

    let mut messages = vec![];

    let refund0 = amounts[0] - unscaled_amount0;
    let refund1 = amounts[1] - unscaled_amount1;
    let mut refund_assets = vec![];
    if !refund0.is_zero() {
        refund_assets.push(Coin::new(
            u128::from(refund0),
            contract_info.base_denom.clone(),
        ));
    }
    if !refund1.is_zero() {
        refund_assets.push(Coin::new(
            u128::from(refund1),
            contract_info.quote_denom.clone(),
        ));
    }
    let mut refund_message: Option<BankMsg> = None;
    if !refund_assets.is_empty() {
        refund_message = Some(BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: refund_assets,
        });
    }

    let scaled_share = convert_to_shares(
        deps.as_ref(),
        env,
        actual_deposits,
        prices,
        [contract_info.base_decimal, contract_info.quote_decimal],
    )?;
    let share = Uint128::new(u128::from(scaled_share.scaled(12)));

    if share.is_zero() {
        return Err(ContractError::CustomError {
            val: format!("Zero share amount"),
        });
    }

    let receiver = addr_opt_validate(deps.api, &receiver)?.unwrap_or_else(|| info.sender.clone());

    let total_share = query_supply(&deps.querier, &contract_info.liquidity_token)?;

    if total_share + share > contract_info.hardcap {
        return Err(ContractError::ExceedHardcap {});
    }

    // Mint LP tokens for the sender or for the receiver (if set)
    messages.extend(mint_liquidity_token_message(
        &contract_info,
        &receiver,
        share,
    )?);

    let mut res = Response::<InjectiveMsgWrapper>::new()
        .add_messages(messages)
        .add_attributes(vec![
            attr("action", "deposit"),
            attr("sender", info.sender),
            attr("receiver", receiver),
            attr(
                "assets",
                format!(
                    "{}, {}",
                    Asset {
                        amount: unscaled_amount0,
                        info: supported[0].clone(),
                    },
                    Asset {
                        amount: unscaled_amount1,
                        info: supported[1].clone(),
                    }
                ),
            ),
            attr("share", share),
        ]);
    match refund_message {
        Some(msg) => res = res.add_message(msg),
        None => {}
    }
    Ok(res)
}

fn try_swap(
    deps: DepsMut<InjectiveQueryWrapper>,
    env: Env,
    info: MessageInfo,
    buying: bool,
    quantity: FPDecimal,
    price: FPDecimal,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;

    if !is_owner(deps.storage, &info.sender)? {
        return Err(ContractError::Unauthorized {});
    }

    let contract = env.contract.address;
    let subaccount_id = contract_info.contract_subaccount_id;
    let min_amount = price * quantity;
    if !info.funds.is_empty() {
        return Err(ContractError::CustomError {
            val: "Do not provide funds!".to_string(),
        });
    }
    let source_denom = if buying {
        contract_info.quote_denom
    } else {
        contract_info.base_denom
    };
    let fee_collected = if buying {
        QUOTE_FEE_COLLECTED.load(deps.storage)?
    } else {
        BASE_FEE_COLLECTED.load(deps.storage)?
    };
    let balance = FPDecimal::from(
        query_balance(&deps.querier, contract.to_string(), source_denom)? - fee_collected,
    );
    if balance < min_amount {
        return Err(ContractError::CustomError {
            val: format!("Swap: {balance} below min_amount: {min_amount}"),
        });
    }
    let order_type = if buying {
        OrderType::Buy
    } else {
        OrderType::Sell
    };
    let order = SpotOrder::new(
        price,
        quantity,
        order_type,
        &contract_info.market_id,
        subaccount_id.clone(),
        Some(contract.to_owned()),
    );

    let order_message = SubMsg::reply_on_success(
        create_batch_update_orders_msg(
            contract,
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![order],
            vec![],
        ),
        ORDER_REPLY_ID,
    );
    let response = Response::<InjectiveMsgWrapper>::new().add_submessage(order_message);

    Ok(response)
}

fn try_cancel_order(
    deps: DepsMut<InjectiveQueryWrapper>,
    env: Env,
    info: MessageInfo,
    order_hash: String,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;

    if !is_owner(deps.storage, &info.sender)? {
        return Err(ContractError::Unauthorized {});
    }

    let contract = env.contract.address;
    let subaccount_id = contract_info.contract_subaccount_id;

    let cancel_message = cancel_spot_order_msg(
        contract,
        contract_info.market_id.clone(),
        subaccount_id.clone(),
        order_hash,
    );
    let response = Response::<InjectiveMsgWrapper>::new().add_message(cancel_message);

    Ok(response)
}

fn add_fee(
    deps: DepsMut<InjectiveQueryWrapper>,
    _env: Env,
    info: MessageInfo,
    base_fee: Uint128,
    quote_fee: Uint128,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    if !is_owner(deps.storage, &info.sender)? {
        return Err(ContractError::Unauthorized {});
    }

    let base_fee_collected = BASE_FEE_COLLECTED.load(deps.storage)?;
    let quote_fee_collected = QUOTE_FEE_COLLECTED.load(deps.storage)?;

    BASE_FEE_COLLECTED.save(deps.storage, &(base_fee_collected + base_fee))?;
    QUOTE_FEE_COLLECTED.save(deps.storage, &(quote_fee_collected + quote_fee))?;

    Ok(Response::default())
}

fn withdraw_fee(
    deps: DepsMut<InjectiveQueryWrapper>,
    _env: Env,
    info: MessageInfo,
    base_fee: Uint128,
    quote_fee: Uint128,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;

    if !is_owner(deps.storage, &info.sender)? {
        return Err(ContractError::Unauthorized {});
    }
    if base_fee.is_zero() && quote_fee.is_zero() {
        return Err(ContractError::CustomError {
            val: format!("Can't withdraw zero fees"),
        });
    }

    let base_fee_collected = BASE_FEE_COLLECTED.load(deps.storage)?;
    let quote_fee_collected = QUOTE_FEE_COLLECTED.load(deps.storage)?;
    if base_fee_collected < base_fee || quote_fee_collected < quote_fee {
        return Err(ContractError::CustomError {
            val: format!("Insufficient fee accrued"),
        });
    }

    BASE_FEE_COLLECTED.save(deps.storage, &(base_fee_collected - base_fee))?;
    QUOTE_FEE_COLLECTED.save(deps.storage, &(quote_fee_collected - quote_fee))?;

    let mut fees: Vec<Coin> = vec![];
    if !base_fee.is_zero() {
        fees.push(Coin::new(
            u128::from(base_fee),
            contract_info.base_denom.clone(),
        ));
    }
    if !quote_fee.is_zero() {
        fees.push(Coin::new(
            u128::from(quote_fee),
            contract_info.quote_denom.clone(),
        ));
    }

    let msgs = vec![BankMsg::Send {
        to_address: info.sender.to_string(),
        amount: fees,
    }];

    Ok(Response::default().add_messages(msgs).add_attribute(
        "fee_withdrawn",
        format!(
            "{}, {}",
            Asset {
                amount: base_fee,
                info: AssetInfo {
                    denom: contract_info.base_denom
                },
            },
            Asset {
                amount: quote_fee,
                info: AssetInfo {
                    denom: contract_info.quote_denom
                },
            }
        ),
    ))
}

/// Mint LP tokens for a beneficiary.
///
/// * **recipient** is the LP token recipient.
///
/// * **amount** is the amount of LP tokens that will be minted for the recipient.
fn mint_liquidity_token_message(
    contract_info: &ContractInfo,
    recipient: &Addr,
    amount: Uint128,
) -> Result<Vec<CosmosMsg<InjectiveMsgWrapper>>, ContractError> {
    let lp_token = &contract_info.liquidity_token;

    return Ok(vec![CosmosMsg::<InjectiveMsgWrapper>::Wasm(
        WasmMsg::Execute {
            contract_addr: lp_token.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Mint {
                recipient: recipient.to_string(),
                amount,
            })?,
            funds: vec![],
        },
    )]);
}

/// Withdraw tokens from the pool.
/// * **sender** is the address that will receive assets back from the vault contract.
///
/// * **share_amount** is the amount of LP tokens to burn.
fn withdraw(
    deps: DepsMut<InjectiveQueryWrapper>,
    env: Env,
    info: MessageInfo,
    sender: Addr,
    share_amount: Uint128,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;

    if info.sender != contract_info.liquidity_token {
        return Err(ContractError::Unauthorized {});
    }
    if share_amount.is_zero() {
        return Err(ContractError::CustomError {
            val: format!("Can't withdraw zero amount"),
        });
    }

    let total_share = query_supply(&deps.querier, &contract_info.liquidity_token)?;
    let refund_assets = get_share_in_assets(deps.as_ref(), env, share_amount, total_share)?;

    let mut messages: Vec<CosmosMsg<InjectiveMsgWrapper>> =
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: contract_info.liquidity_token.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Burn {
                amount: share_amount,
            })?,
            funds: vec![],
        })];
    if !refund_assets[0].amount.is_zero() {
        messages.push(refund_assets[0].clone().into_msg(sender.clone())?);
    }
    if !refund_assets[1].amount.is_zero() {
        messages.push(refund_assets[1].clone().into_msg(sender.clone())?);
    }
    if !refund_assets[2].amount.is_zero() {
        messages.push(refund_assets[2].clone().into_msg(sender.clone())?);
    }

    Ok(Response::<InjectiveMsgWrapper>::new()
        .add_messages(messages)
        .add_attributes(vec![
            attr("action", "withdraw"),
            attr("sender", sender),
            attr("withdrawn_share", share_amount),
            attr(
                "refund_assets",
                format!("{}, {}", refund_assets[0], refund_assets[1]),
            ),
        ]))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps<InjectiveQueryWrapper>, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Ownership {} => to_binary(&get_ownership(deps.storage)?),
        QueryMsg::TokensForShares { share } => to_binary(&get_tokens_for_shares(deps, env, share)?),
        QueryMsg::TotalLiquidity {} => to_binary(&get_total_liquidity(deps, env)?),
        QueryMsg::UserLiquidity { user } => to_binary(&get_user_liquidity(deps, env, user)?),
        QueryMsg::Prices {} => to_binary(&query_prices(deps, env)?),
        QueryMsg::Tokens {} => to_binary(&query_tokens(deps.storage)?),
    }
}

fn get_tokens_for_shares(
    deps: Deps<InjectiveQueryWrapper>,
    env: Env,
    share: Uint128,
) -> StdResult<[Uint128; 2]> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;
    let balance0 = query_balance(
        &deps.querier,
        env.contract.address.to_string(),
        &contract_info.base_denom,
    )? - BASE_FEE_COLLECTED.load(deps.storage)?;
    let balance1 = query_balance(
        &deps.querier,
        env.contract.address.to_string(),
        &contract_info.quote_denom,
    )? - QUOTE_FEE_COLLECTED.load(deps.storage)?;

    let total_share = query_supply(&deps.querier, &contract_info.liquidity_token)?;

    let asset0 = balance0 * share / total_share;
    let asset1 = balance1 * share / total_share;

    Ok([asset0, asset1])
}

fn get_total_liquidity(deps: Deps<InjectiveQueryWrapper>, env: Env) -> StdResult<[Uint128; 2]> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;
    let balance0 = query_balance(
        &deps.querier,
        env.contract.address.to_string(),
        &contract_info.base_denom,
    )? - BASE_FEE_COLLECTED.load(deps.storage)?;
    let balance1 = query_balance(
        &deps.querier,
        env.contract.address.to_string(),
        &contract_info.quote_denom,
    )? - QUOTE_FEE_COLLECTED.load(deps.storage)?;

    Ok([balance0, balance1])
}

fn get_user_liquidity(deps: Deps<InjectiveQueryWrapper>, env: Env, user: String) -> StdResult<[Asset; 2]> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;
    let total_share = query_supply(&deps.querier, &contract_info.liquidity_token)?;
    let share = query_token_balance(&deps.querier, &contract_info.liquidity_token, user)?;
    let balance0 = query_balance(
        &deps.querier,
        env.contract.address.to_string(),
        &contract_info.base_denom,
    )? - BASE_FEE_COLLECTED.load(deps.storage)?;
    let balance1 = query_balance(
        &deps.querier,
        env.contract.address.to_string(),
        &contract_info.quote_denom,
    )? - QUOTE_FEE_COLLECTED.load(deps.storage)?;
    let liquidity0 = balance0 * share / total_share;
    let liquidity1 = balance1 * share / total_share;

    Ok([
        Asset {
            amount: liquidity0,
            info: AssetInfo {
                denom: contract_info.base_denom.clone(),
            },
        },
        Asset {
            amount: liquidity1,
            info: AssetInfo {
                denom: contract_info.quote_denom.clone(),
            },
        },
    ])
}

pub fn query_tokens(storage: &dyn Storage) -> StdResult<[String; 2]> {
    let contract_info = CONTRACT_INFO.load(storage)?;

    Ok([contract_info.base_denom, contract_info.quote_denom])
}

fn convert_to_shares(
    deps: Deps<InjectiveQueryWrapper>,
    env: Env,
    amounts: [FPDecimal; 2],
    prices: [FPDecimal; 2],
    decimals: [u8; 2],
) -> StdResult<FPDecimal> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;

    let total_share =
        FPDecimal::from(query_supply(&deps.querier, &contract_info.liquidity_token)?).scaled(-12);
    let total_deposit_value = amounts[0] * prices[0] + amounts[1] * prices[1];
    let share = if total_share.is_zero() {
        total_deposit_value
    } else {
        let balance0 = FPDecimal::from(
            query_balance(
                &deps.querier,
                env.contract.address.to_string(),
                contract_info.base_denom,
            )? - BASE_FEE_COLLECTED.load(deps.storage)?,
        )
        .scaled(-(decimals[0] as i32));
        let balance1 = FPDecimal::from(
            query_balance(
                &deps.querier,
                env.contract.address.to_string(),
                contract_info.quote_denom,
            )? - QUOTE_FEE_COLLECTED.load(deps.storage)?,
        )
        .scaled(-(decimals[1] as i32));
        let total_value = balance0 * prices[0] + balance1 * prices[1];
        total_share * total_deposit_value / total_value
    };

    Ok(share)
}

fn get_share_in_assets(
    deps: Deps<InjectiveQueryWrapper>,
    env: Env,
    share: Uint128,
    total_share: Uint128,
) -> StdResult<[Asset; 3]> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;
    let balance0 = query_balance(
        &deps.querier,
        env.contract.address.to_string(),
        &contract_info.base_denom,
    )? - BASE_FEE_COLLECTED.load(deps.storage)?;
    let balance1 = query_balance(
        &deps.querier,
        env.contract.address.to_string(),
        &contract_info.quote_denom,
    )? - QUOTE_FEE_COLLECTED.load(deps.storage)?;
    let refund_amount0 = balance0 * share / total_share;
    let refund_amount1 = balance1 * share / total_share;
    let mut fee_amount = Uint128::zero();
    let fee_denom = "INJ".to_string();
    if contract_info.base_denom != fee_denom && contract_info.quote_denom != fee_denom {
        let inj_balance: Uint128 =
            query_balance(&deps.querier, env.contract.address.to_string(), &fee_denom)?;
        fee_amount = inj_balance * share / total_share;
    }
    Ok([
        Asset {
            amount: refund_amount0,
            info: AssetInfo {
                denom: contract_info.base_denom.clone(),
            },
        },
        Asset {
            amount: refund_amount1,
            info: AssetInfo {
                denom: contract_info.quote_denom.clone(),
            },
        },
        Asset {
            amount: fee_amount,
            info: AssetInfo {
                denom: fee_denom.clone(),
            },
        },
    ])
}

fn query_prices(deps: Deps<InjectiveQueryWrapper>, env: Env) -> StdResult<[Uint128; 2]> {
    let prices: [FPDecimal; 2] = get_prices(deps, env)?;

    Ok([
        Uint128::new(u128::from(prices[0].scaled(8))),
        Uint128::new(u128::from(prices[1].scaled(8))),
    ])
}

fn get_prices(deps: Deps<InjectiveQueryWrapper>, env: Env) -> StdResult<[FPDecimal; 2]> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;
    let querier = InjectiveQuerier::new(&deps.querier);
    let response0 = querier.query_pyth_price(contract_info.base_price_id.as_str())?;
    let response1 = querier.query_pyth_price(contract_info.quote_price_id.as_str())?;
    let base_price_state = response0
        .price_state
        .expect("Failed to get base asset price")
        .price_state;
    let base_price = base_price_state.price;
    let quote_price_state = response1
        .price_state
        .expect("Failed to get quote asset price")
        .price_state;
    let quote_price = quote_price_state.price;

    let timestamp = env.block.time.seconds() as i64;
    if base_price_state.timestamp < timestamp - PRICE_VALID_DURATION {
        return Err(StdError::GenericErr {
            msg: "Price too old".to_owned(),
        });
    }
    if quote_price_state.timestamp < timestamp - PRICE_VALID_DURATION {
        return Err(StdError::GenericErr {
            msg: "Price too old".to_owned(),
        });
    }

    Ok([base_price, quote_price])
}
