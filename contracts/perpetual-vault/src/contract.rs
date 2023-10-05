use cosmwasm_std::entry_point;
use cosmwasm_std::{
    attr, from_binary, to_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env,
    MessageInfo, Reply, ReplyOn, Response, StdError, StdResult, SubMsg, Uint128, WasmMsg, Storage
};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg, MinterResponse};
use cw20_base::msg::InstantiateMsg as TokenInstantiateMsg;
use cw_ownable::{get_ownership, is_owner, update_ownership};
use injective_math::scale::Scaled;
use injective_math::FPDecimal;
use injective_protobuf::proto::tx;
use protobuf::Message;
use std::str::FromStr;

use injective_cosmwasm::{
    cancel_derivative_order_msg, create_derivative_market_order_msg,
    get_default_subaccount_id_for_checked_address, DerivativeOrder, InjectiveMsgWrapper,
    InjectiveQuerier, InjectiveQueryWrapper, MarketStatus, OrderType,
};

use crate::asset::{addr_opt_validate, format_lp_token_name, Asset, AssetInfo, CoinsExt};
use crate::error::ContractError;
use crate::msg::{Cw20HookMsg, ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::querier::{query_balance, query_supply, query_token_balance};
use crate::response::MsgInstantiateContractResponse;
use crate::state::{ContractInfo, CONTRACT_INFO, FEE_COLLECTED};

/// A `reply` call code ID used for sub-messages.
pub const INSTANTIATE_TOKEN_REPLY_ID: u64 = 1u64;
pub const ORDER_REPLY_ID: u64 = 2u64;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut<InjectiveQueryWrapper>,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    let querier = InjectiveQuerier::new(&deps.querier);
    if let Some(full_market) = querier.query_derivative_market(&msg.market_id)?.market {
        if let Some(market) = full_market.market {
            if market.status != MarketStatus::Active {
                return Err(ContractError::CustomError {
                    val: format!("Market with id: {} not active", msg.market_id.as_str()),
                });
            }
            cw_ownable::initialize_owner(deps.storage, deps.api, Some(msg.owner.as_str()))
                .expect(format!("Invalid owner: {}", msg.owner).as_str());
            let contract_info = ContractInfo {
                market_id: msg.market_id,
                quote_denom: market.quote_denom,
                quote_decimal: msg.quote_decimal,
                hardcap: msg.hardcap,
                liquidity_token: Addr::unchecked(""),
                contract_subaccount_id: get_default_subaccount_id_for_checked_address(
                    &env.contract.address,
                ),
            };
            CONTRACT_INFO.save(deps.storage, &contract_info)?;
            FEE_COLLECTED.save(deps.storage, &Uint128::zero())?;
            let token_name = format_lp_token_name(&contract_info.quote_denom)?;

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
    let dec_scale_factor: FPDecimal = FPDecimal::from(1000000000000000000_i128);
    let id = msg.id;
    let order_response: tx::MsgCreateDerivativeMarketOrderResponse = Message::parse_from_bytes(
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

    // unwrap results into trade_data
    let trade_data = match order_response.results.into_option() {
        Some(trade_data) => Ok(trade_data),
        None => Err(ContractError::CustomError {
            val: "No trade data in order response".to_string(),
        }),
    }?;
    let quantity = FPDecimal::from_str(&trade_data.quantity)? / dec_scale_factor;
    let price = FPDecimal::from_str(&trade_data.price)? / dec_scale_factor;
    let fee = FPDecimal::from_str(&trade_data.fee)? / dec_scale_factor;

    Ok(Response::new().add_attributes(vec![
        attr("action", "swap"),
        attr("order_hash", order_response.order_hash),
        attr("quantity", Uint128::from(u128::from(quantity))),
        attr("price", Uint128::from(u128::from(price))),
        attr("fee", Uint128::from(u128::from(fee))),
    ]))
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
        ExecuteMsg::SwapPerpetual {
            long,
            quantity,
            price,
            margin,
        } => try_swap(deps, env, info, long, quantity, price, margin),
        ExecuteMsg::CancelOrder { order_hash } => try_cancel_order(deps, env, info, order_hash),
        ExecuteMsg::AddFee { fee } => add_fee(deps, env, info, fee),
        ExecuteMsg::WithdrawFee { fee } => withdraw_fee(deps, env, info, fee),
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
///
/// NOTE - the address that wants to deposit should approve the vault contract to pull its relevant tokens.
fn deposit(
    deps: DepsMut<InjectiveQueryWrapper>,
    env: Env,
    info: MessageInfo,
    assets: Vec<Asset>,
    receiver: Option<String>,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    if assets.len() != 1 {
        return Err(StdError::generic_err("assets must contain exactly one element").into());
    }
    assets[0].info.check(deps.api)?;

    let contract_info = CONTRACT_INFO.load(deps.storage)?;

    let supported = vec![AssetInfo {
        denom: contract_info.quote_denom.clone(),
    }];
    info.funds.assert_coins_properly_sent(&assets, &supported)?;

    let amount = assets
        .iter()
        .find(|a| a.info.equal(&supported[0]))
        .map(|a| a.amount)
        .expect("Wrong asset info is given");

    let scaled_amount = FPDecimal::from(amount).scaled(-(contract_info.quote_decimal as i32));

    if scaled_amount.is_zero() {
        return Err(ContractError::InvalidZeroAmount {});
    }

    let mut messages = vec![];

    let _share = convert_to_shares(
        deps.as_ref(),
        env,
        scaled_amount,
        contract_info.quote_decimal,
    )?;
    let share = Uint128::new(u128::from(_share.scaled(12)));

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

    let res = Response::<InjectiveMsgWrapper>::new()
        .add_messages(messages)
        .add_attributes(vec![
            attr("action", "deposit"),
            attr("sender", info.sender),
            attr("receiver", receiver),
            attr(
                "assets",
                format!(
                    "{}",
                    Asset {
                        amount: amount,
                        info: supported[0].clone(),
                    },
                ),
            ),
            attr("share", share),
        ]);
    Ok(res)
}

fn try_swap(
    deps: DepsMut<InjectiveQueryWrapper>,
    env: Env,
    info: MessageInfo,
    long: bool,
    quantity: FPDecimal,
    price: FPDecimal,
    margin: FPDecimal,
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
    let denom = contract_info.quote_denom;
    let fee_collected = FEE_COLLECTED.load(deps.storage)?;
    let balance =
        FPDecimal::from(query_balance(&deps.querier, contract.to_string(), denom)? - fee_collected);
    if balance < min_amount {
        return Err(ContractError::CustomError {
            val: format!("Swap: {balance} below min_amount: {min_amount}"),
        });
    }
    let order_type = if long {
        OrderType::Buy
    } else {
        OrderType::Sell
    };
    let order = DerivativeOrder::new(
        price,
        quantity,
        margin,
        order_type,
        contract_info.market_id,
        subaccount_id,
        Some(contract.to_owned()),
    );

    let order_message = SubMsg::reply_on_success(
        create_derivative_market_order_msg(contract, order),
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

    let cancel_message = cancel_derivative_order_msg(
        contract,
        contract_info.market_id.clone(),
        subaccount_id.clone(),
        order_hash,
        1,
    );
    let response = Response::<InjectiveMsgWrapper>::new().add_message(cancel_message);

    Ok(response)
}

fn add_fee(
    deps: DepsMut<InjectiveQueryWrapper>,
    _env: Env,
    info: MessageInfo,
    fee: Uint128,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    if !is_owner(deps.storage, &info.sender)? {
        return Err(ContractError::Unauthorized {});
    }

    let fee_collected = FEE_COLLECTED.load(deps.storage)?;

    FEE_COLLECTED.save(deps.storage, &(fee_collected + fee))?;

    Ok(Response::default())
}

fn withdraw_fee(
    deps: DepsMut<InjectiveQueryWrapper>,
    _env: Env,
    info: MessageInfo,
    fee: Uint128,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;

    if !is_owner(deps.storage, &info.sender)? {
        return Err(ContractError::Unauthorized {});
    }
    if fee.is_zero() {
        return Err(ContractError::CustomError {
            val: format!("Can't withdraw zero fees"),
        });
    }

    let fee_collected = FEE_COLLECTED.load(deps.storage)?;
    if fee_collected < fee {
        return Err(ContractError::CustomError {
            val: format!("Insufficient fee accrued"),
        });
    }

    FEE_COLLECTED.save(deps.storage, &(fee_collected - fee))?;

    let fees = vec![Coin::new(
        u128::from(fee),
        contract_info.quote_denom.clone(),
    )];
    let msgs = vec![BankMsg::Send {
        to_address: info.sender.to_string(),
        amount: fees,
    }];

    Ok(Response::default().add_messages(msgs).add_attribute(
        "fee_withdrawn",
        format!(
            "{}",
            Asset {
                amount: fee,
                info: AssetInfo {
                    denom: contract_info.quote_denom
                },
            }
        ),
    ))
}

/// Mint LP tokens for a beneficiary and auto stake the tokens in the Generator contract (if auto staking is specified).
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

/// Withdraw liquidity from the pool.
/// * **sender** is the address that will receive assets back from the pair contract.
///
/// * **amount** is the amount of LP tokens to burn.
fn withdraw(
    deps: DepsMut<InjectiveQueryWrapper>,
    env: Env,
    info: MessageInfo,
    sender: Addr,
    amount: Uint128,
) -> Result<Response<InjectiveMsgWrapper>, ContractError> {
    let contract_info = CONTRACT_INFO
        .load(deps.storage)
        .expect("failed to load contract info");

    if info.sender != contract_info.liquidity_token {
        return Err(ContractError::Unauthorized {});
    }
    if amount.is_zero() {
        return Err(ContractError::CustomError {
            val: format!("Can't withdraw zero amount"),
        });
    }

    let total_share = query_supply(&deps.querier, &contract_info.liquidity_token)?;
    let refund_assets = get_share_in_assets(deps.as_ref(), env, amount, total_share)?;

    let mut messages: Vec<CosmosMsg<InjectiveMsgWrapper>> =
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: contract_info.liquidity_token.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Burn { amount })?,
            funds: vec![],
        })];
    if !refund_assets[0].amount.is_zero() {
        messages.push(refund_assets[0].clone().into_msg(sender.clone())?);
    }
    if !refund_assets[1].amount.is_zero() {
        messages.push(refund_assets[1].clone().into_msg(sender.clone())?);
    }

    Ok(Response::<InjectiveMsgWrapper>::new()
        .add_messages(messages)
        .add_attributes(vec![
            attr("action", "withdraw"),
            attr("sender", sender),
            attr("withdrawn_share", amount),
            attr("refund_assets", format!("{}", refund_assets[0])),
        ]))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps<InjectiveQueryWrapper>, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Ownership {} => to_binary(&get_ownership(deps.storage)?),
        QueryMsg::TokensForShares { share } => to_binary(&get_tokens_for_shares(deps, env, share)?),
        QueryMsg::TotalLiquidity {} => to_binary(&get_total_liquidity(deps, env)?),
        QueryMsg::UserLiquidity { user } => to_binary(&get_user_liquidity(deps, env, user)?),
        QueryMsg::Tokens {} => to_binary(&query_tokens(deps.storage)?),
    }
}

fn get_tokens_for_shares(
    deps: Deps<InjectiveQueryWrapper>,
    env: Env,
    share: Uint128,
) -> StdResult<Uint128> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;
    let balance = query_balance(
        &deps.querier,
        env.contract.address.to_string(),
        &contract_info.quote_denom,
    )? - FEE_COLLECTED.load(deps.storage)?;

    let total_share = query_supply(&deps.querier, &contract_info.liquidity_token)?;

    let asset = balance * share / total_share;

    Ok(asset)
}

fn get_total_liquidity(deps: Deps<InjectiveQueryWrapper>, env: Env) -> StdResult<Uint128> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;
    let balance = query_balance(
        &deps.querier,
        env.contract.address.to_string(),
        &contract_info.quote_denom,
    )? - FEE_COLLECTED.load(deps.storage)?;

    Ok(balance)
}

fn get_user_liquidity(
    deps: Deps<InjectiveQueryWrapper>,
    env: Env,
    user: String,
) -> StdResult<[Asset; 1]> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;
    let total_share = query_supply(&deps.querier, &contract_info.liquidity_token)?;
    let share = query_token_balance(&deps.querier, &contract_info.liquidity_token, user)?;
    let balance = query_balance(
        &deps.querier,
        env.contract.address.to_string(),
        &contract_info.quote_denom,
    )? - FEE_COLLECTED.load(deps.storage)?;
    let liquidity = balance * share / total_share;

    Ok([
        Asset {
            amount: liquidity,
            info: AssetInfo {
                denom: contract_info.quote_denom.clone(),
            },
        },
    ])
}

pub fn query_tokens(storage: &dyn Storage) -> StdResult<[String; 1]> {
    let contract_info = CONTRACT_INFO.load(storage)?;

    Ok([contract_info.quote_denom])
}

fn convert_to_shares(
    deps: Deps<InjectiveQueryWrapper>,
    env: Env,
    amount: FPDecimal,
    decimal: u8,
) -> StdResult<FPDecimal> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;

    let total_share =
        FPDecimal::from(query_supply(&deps.querier, &contract_info.liquidity_token)?).scaled(-12);
    let share = if total_share.is_zero() {
        amount
    } else {
        let balance = FPDecimal::from(
            query_balance(
                &deps.querier,
                env.contract.address.to_string(),
                contract_info.quote_denom,
            )? - FEE_COLLECTED.load(deps.storage)?,
        )
        .scaled(-(decimal as i32));
        total_share * amount / balance
    };

    Ok(share)
}

fn get_share_in_assets(
    deps: Deps<InjectiveQueryWrapper>,
    env: Env,
    share: Uint128,
    total_share: Uint128,
) -> StdResult<[Asset; 2]> {
    let contract_info = CONTRACT_INFO.load(deps.storage)?;
    let balance = query_balance(
        &deps.querier,
        env.contract.address.to_string(),
        &contract_info.quote_denom,
    )? - FEE_COLLECTED.load(deps.storage)?;
    let refund_amount = balance * share / total_share;
    let mut fee_amount = Uint128::zero();
    let fee_denom = "INJ".to_string();
    if contract_info.quote_denom != fee_denom {
        let inj_balance: Uint128 =
            query_balance(&deps.querier, env.contract.address.to_string(), &fee_denom)?;
        fee_amount = inj_balance * share / total_share;
    }
    Ok([
        Asset {
            amount: refund_amount,
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
