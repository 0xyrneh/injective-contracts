use std::str::FromStr;

use cosmwasm_std::testing::{mock_info, MockApi, MockStorage};
use cosmwasm_std::{
    attr, to_binary, BankMsg, Binary, Coin, ContractResult, DepsMut, OwnedDeps, QuerierResult,
    Reply, ReplyOn, StdError, SubMsg, SubMsgResponse, SubMsgResult, SystemResult, Uint128, WasmMsg,
};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg, MinterResponse};
use cw20_base::msg::InstantiateMsg as TokenInstantiateMsg;
use injective_cosmwasm::oracle::types::{PriceState, PythPriceState};
use injective_cosmwasm::InjectiveMsg::BatchUpdateOrders;
use injective_cosmwasm::{
    exchange::spot::{ShortOrderInfo, ShortSpotOrder},
    inj_mock_env, HandlesMarketIdQuery, HandlesPythPriceQuery, InjectiveQueryWrapper,
    InjectiveRoute, MarketId, MarketStatus, OrderType, PythPriceResponse, SpotMarket,
    SpotMarketResponse, SubaccountId,
};
use injective_math::FPDecimal;
use protobuf::Message;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::asset::{Asset, AssetInfo};
use crate::contract::{execute, instantiate, reply, ORDER_REPLY_ID};
use crate::error::ContractError;
use crate::helpers::{get_message_data, i32_to_dec};
use crate::msg::{Cw20HookMsg, ExecuteMsg, InstantiateMsg};
use crate::response::MsgInstantiateContractResponse;
use crate::state::CONTRACT_INFO;
use crate::test::mock_querier::{mock_dependencies, WasmMockQuerier};

const TEST_CONTRACT_ADDR: &str = "inj14hj2tavq8fpesdwxxcu44rty3hh90vhujaxlnz";

const TEST_MARKET_ID: &str = "0x78c2d3af98c517b164070a739681d4bd4d293101e7ffc3a30968945329b47ec6";

fn test_deps<'a>() -> OwnedDeps<MockStorage, MockApi, WasmMockQuerier, InjectiveQueryWrapper> {
    mock_dependencies(&[], |querier| {
        querier.pyth_price_response_handler = Some(Box::new(create_pyth_price_handler()));
        querier.spot_market_response_handler = Some(Box::new(create_spot_market_handler()));
    })
}

fn store_liquidity_token(deps: DepsMut<InjectiveQueryWrapper>, msg_id: u64, contract_addr: String) {
    let data = MsgInstantiateContractResponse {
        contract_address: contract_addr,
        data: vec![],
        unknown_fields: Default::default(),
        cached_size: Default::default(),
    }
    .write_to_bytes()
    .expect("failed to convert to bytes array");

    let reply_msg = Reply {
        id: msg_id,
        result: SubMsgResult::Ok(SubMsgResponse {
            events: vec![],
            data: Some(data.into()),
        }),
    };

    let _res = reply(deps, inj_mock_env(), reply_msg.clone()).expect("failed to reply");
}

#[test]
fn proper_initialization() {
    let mut deps = test_deps();

    deps.querier.with_token_balances(&[
        (
            &"asset0000".to_string(),
            &[(&String::from(TEST_CONTRACT_ADDR), &Uint128::new(0))],
        ),
        (
            &"asset0001".to_string(),
            &[(&String::from(TEST_CONTRACT_ADDR), &Uint128::new(0))],
        ),
    ]);

    // Fail to initialize when market does not exist
    let msg = InstantiateMsg {
        owner: "addr0000".to_string(),
        market_id: MarketId::new(
            "0x0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        )
        .expect("failed to create market_id"),
        base_decimal: 18,
        quote_decimal: 6,
        base_price_id: "INJ_PRICE_ID".to_string(),
        quote_price_id: "USDT_PRICE_ID".to_string(),
        hardcap: Uint128::new(5000_000000000000u128),
        token_code_id: 10u64,
    };

    let env = inj_mock_env();
    let info = mock_info("addr0000", &[]);
    let res = instantiate(deps.as_mut(), env, info, msg.clone()).unwrap_err();
    assert_eq!(
        res,
        ContractError::CustomError {
            val: format!("Market with id: {} not found", msg.market_id.as_str()),
        }
    );

    // Initialize
    let msg = InstantiateMsg {
        owner: "addr0000".to_string(),
        market_id: MarketId::new(TEST_MARKET_ID.to_string()).expect("failed to create market_id"),
        base_decimal: 18,
        quote_decimal: 6,
        base_price_id: "INJ_PRICE_ID".to_string(),
        quote_price_id: "USDT_PRICE_ID".to_string(),
        hardcap: Uint128::new(5000_000000000000u128),
        token_code_id: 10u64,
    };

    let env = inj_mock_env();
    let info = mock_info("addr0000", &[]);
    let res = instantiate(deps.as_mut(), env, info, msg).expect("failed to instantiate");
    assert_eq!(
        res.messages,
        vec![SubMsg {
            msg: WasmMsg::Instantiate {
                code_id: 10u64,
                msg: to_binary(&TokenInstantiateMsg {
                    name: "INJ-USDT-LP".to_string(),
                    symbol: "uLP".to_string(),
                    decimals: 12,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: String::from(TEST_CONTRACT_ADDR),
                        cap: None,
                    }),
                    marketing: None
                })
                .expect("failed to convert to binary"),
                funds: vec![],
                admin: None,
                label: String::from("Elixir LP token"),
            }
            .into(),
            id: 1,
            gas_limit: None,
            reply_on: ReplyOn::Success
        },]
    );

    // Store liquidity token
    store_liquidity_token(deps.as_mut(), 1, "liquidity0000".to_string());

    let contract_info = CONTRACT_INFO
        .load(deps.as_ref().storage)
        .expect("failed to load contract info");
    assert_eq!("INJ".to_string(), contract_info.base_denom);
    assert_eq!("USDT".to_string(), contract_info.quote_denom);
    assert_eq!(18, contract_info.base_decimal);
    assert_eq!(6, contract_info.quote_decimal);
    assert_eq!("liquidity0000".to_string(), contract_info.liquidity_token);
}

#[test]
fn deposit() {
    let mut deps = test_deps();

    deps.querier.with_token_balances(&[
        (
            &"asset0000".to_string(),
            &[(&String::from(TEST_CONTRACT_ADDR), &Uint128::new(0))],
        ),
        (
            &"asset0001".to_string(),
            &[(&String::from(TEST_CONTRACT_ADDR), &Uint128::new(0))],
        ),
        (
            &"liquidity0000".to_string(),
            &[(&String::from(TEST_CONTRACT_ADDR), &Uint128::new(0))],
        ),
    ]);

    let msg = InstantiateMsg {
        owner: "addr0000".to_string(),
        market_id: MarketId::new(TEST_MARKET_ID.to_string()).expect("failed to create market_id"),
        base_decimal: 18,
        quote_decimal: 6,
        base_price_id: "INJ_PRICE_ID".to_string(),
        quote_price_id: "USDT_PRICE_ID".to_string(),
        hardcap: Uint128::new(5000_000000000000u128),
        token_code_id: 10u64,
    };

    let env = inj_mock_env();
    let info = mock_info("addr0000", &[]);
    let _res = instantiate(deps.as_mut(), env, info, msg).expect("failed to instantiate");

    // Store liquidity token
    store_liquidity_token(deps.as_mut(), 1, "liquidity0000".to_string());

    // Fail to deposit when wrong number of assets provided
    let msg = ExecuteMsg::Deposit {
        assets: vec![
            Asset {
                info: AssetInfo {
                    denom: "INJ".to_string(),
                },
                amount: Uint128::from(10_000000000000000000u128),
            },
            Asset {
                info: AssetInfo {
                    denom: "USDT".to_string(),
                },
                amount: Uint128::from(100_000000u128),
            },
            Asset {
                info: AssetInfo {
                    denom: "USDC".to_string(),
                },
                amount: Uint128::from(100_000000u128),
            },
        ],
        receiver: None,
    };

    let env = inj_mock_env();
    let info = mock_info("addr0001", &[]);
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(
        res,
        StdError::generic_err("assets must contain exactly two elements").into()
    );

    // Fail to deposit when wrong assets provided
    let msg = ExecuteMsg::Deposit {
        assets: vec![
            Asset {
                info: AssetInfo {
                    denom: "INJ".to_string(),
                },
                amount: Uint128::from(10_000000000000000000u128),
            },
            Asset {
                info: AssetInfo {
                    denom: "USDC".to_string(),
                },
                amount: Uint128::from(100_000000u128),
            },
        ],
        receiver: None,
    };

    let env = inj_mock_env();
    let info = mock_info("addr0001", &[]);
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(
        res,
        StdError::generic_err("Asset USDC is not in the pool").into()
    );

    // Fail to deposit when assets amount mismatch
    let msg = ExecuteMsg::Deposit {
        assets: vec![
            Asset {
                info: AssetInfo {
                    denom: "INJ".to_string(),
                },
                amount: Uint128::from(10_000000000000000000u128),
            },
            Asset {
                info: AssetInfo {
                    denom: "USDT".to_string(),
                },
                amount: Uint128::from(120_000000u128),
            },
        ],
        receiver: None,
    };

    let env = inj_mock_env();
    let info = mock_info(
        "addr0001",
        &[
            Coin {
                denom: "INJ".to_string(),
                amount: Uint128::from(10_000000000000000000u128),
            },
            Coin {
                denom: "USDT".to_string(),
                amount: Uint128::from(100_000000u128),
            },
        ],
    );
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(
        res,
        StdError::generic_err(
            "Native token balance mismatch between the argument and the transferred"
        )
        .into()
    );

    // Fail to deposit when extra asset is provided
    let msg = ExecuteMsg::Deposit {
        assets: vec![
            Asset {
                info: AssetInfo {
                    denom: "INJ".to_string(),
                },
                amount: Uint128::from(10_000000000000000000u128),
            },
            Asset {
                info: AssetInfo {
                    denom: "USDT".to_string(),
                },
                amount: Uint128::from(100_000000u128),
            },
        ],
        receiver: None,
    };

    let env = inj_mock_env();
    let info = mock_info(
        "addr0001",
        &[
            Coin {
                denom: "INJ".to_string(),
                amount: Uint128::from(10_000000000000000000u128),
            },
            Coin {
                denom: "USDT".to_string(),
                amount: Uint128::from(100_000000u128),
            },
            Coin {
                denom: "USDC".to_string(),
                amount: Uint128::from(50_000000u128),
            },
        ],
    );
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(
        res,
        StdError::generic_err("Supplied coins contain USDC that is not in the input asset vector")
            .into()
    );

    // Deposit
    let msg = ExecuteMsg::Deposit {
        assets: vec![
            Asset {
                info: AssetInfo {
                    denom: "INJ".to_string(),
                },
                amount: Uint128::from(10_000000000000000000u128),
            },
            Asset {
                info: AssetInfo {
                    denom: "USDT".to_string(),
                },
                amount: Uint128::from(100_000000u128),
            },
        ],
        receiver: None,
    };

    let env = inj_mock_env();
    let info = mock_info(
        "addr0001",
        &[
            Coin {
                denom: "INJ".to_string(),
                amount: Uint128::from(10_000000000000000000u128),
            },
            Coin {
                denom: "USDT".to_string(),
                amount: Uint128::from(100_000000u128),
            },
        ],
    );
    let res = execute(deps.as_mut(), env.clone(), info, msg).expect("failed to deposit");
    let mint_receiver_msg = res.messages.get(0).expect("no message");
    let refund_msg = res.messages.get(1).expect("no message");
    assert_eq!(
        mint_receiver_msg,
        &SubMsg {
            msg: WasmMsg::Execute {
                contract_addr: String::from("liquidity0000"),
                msg: to_binary(&Cw20ExecuteMsg::Mint {
                    recipient: String::from("addr0001"),
                    amount: Uint128::from(180_000000000000u128),
                })
                .expect("failed to convert to binary"),
                funds: vec![],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
        }
    );
    assert_eq!(
        refund_msg,
        &SubMsg {
            msg: BankMsg::Send {
                to_address: String::from("addr0001"),
                amount: vec![Coin::new(10_000000u128, "USDT",)],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never
        }
    );

    // Fail to deposit 0 amounts
    let msg = ExecuteMsg::Deposit {
        assets: vec![
            Asset {
                info: AssetInfo {
                    denom: "INJ".to_string(),
                },
                amount: Uint128::zero(),
            },
            Asset {
                info: AssetInfo {
                    denom: "USDT".to_string(),
                },
                amount: Uint128::from(100_000000u128),
            },
        ],
        receiver: None,
    };

    let env = inj_mock_env();
    let info = mock_info(
        "addr0001",
        &[
            Coin {
                denom: "INJ".to_string(),
                amount: Uint128::zero(),
            },
            Coin {
                denom: "USDT".to_string(),
                amount: Uint128::from(100_000000u128),
            },
        ],
    );
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(res, ContractError::InvalidZeroAmount {});

    // Fail to deposit more than hardcap
    let msg = ExecuteMsg::Deposit {
        assets: vec![
            Asset {
                info: AssetInfo {
                    denom: "INJ".to_string(),
                },
                amount: Uint128::from(300_000000000000000000u128),
            },
            Asset {
                info: AssetInfo {
                    denom: "USDT".to_string(),
                },
                amount: Uint128::from(2700_000000u128),
            },
        ],
        receiver: None,
    };

    let env = inj_mock_env();
    let info = mock_info(
        "addr0001",
        &[
            Coin {
                denom: "INJ".to_string(),
                amount: Uint128::from(300_000000000000000000u128),
            },
            Coin {
                denom: "USDT".to_string(),
                amount: Uint128::from(2700_000000u128),
            },
        ],
    );
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(res, ContractError::ExceedHardcap {});
}

#[test]
fn withdraw_n_fee() {
    let mut deps = test_deps();

    deps.querier.with_token_balances(&[(
        &"liquidity0000".to_string(),
        &[(
            &String::from("addr0001"),
            &Uint128::new(180_000000000000u128),
        )],
    )]);
    deps.querier.with_balance(&[(
        &String::from(TEST_CONTRACT_ADDR),
        &[
            Coin {
                denom: "INJ".to_string(),
                amount: Uint128::from(10_000000000000000000u128),
            },
            Coin {
                denom: "USDT".to_string(),
                amount: Uint128::from(90_000000u128),
            },
        ],
    )]);

    let msg = InstantiateMsg {
        owner: "addr0000".to_string(),
        market_id: MarketId::new(TEST_MARKET_ID.to_string()).expect("failed to create market_id"),
        base_decimal: 18,
        quote_decimal: 6,
        base_price_id: "INJ_PRICE_ID".to_string(),
        quote_price_id: "USDT_PRICE_ID".to_string(),
        hardcap: Uint128::new(5000_000000000000u128),
        token_code_id: 10u64,
    };

    let env = inj_mock_env();
    let info = mock_info("addr0000", &[]);
    let _res = instantiate(deps.as_mut(), env, info, msg).expect("failed to instantiate");

    // Store liquidity token
    store_liquidity_token(deps.as_mut(), 1, "liquidity0000".to_string());

    // Fail to withdraw when wrong liquidity is provided
    let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender: String::from("addr0001"),
        msg: to_binary(&Cw20HookMsg::Withdraw {}).expect("failed to convert to binary"),
        amount: Uint128::new(90_000000000000u128),
    });

    let env = inj_mock_env();
    let info = mock_info("liquidity0001", &[]);
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(res, ContractError::Unauthorized {});

    // Fail to set fee as non owner
    let msg = ExecuteMsg::AddFee {
        base_fee: Uint128::from(1_000000000000000000u128),
        quote_fee: Uint128::from(9_000000u128),
    };

    let env = inj_mock_env();
    let info = mock_info("addr0001", &[]);
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(res, ContractError::Unauthorized {});

    // Set fee as owner
    let msg = ExecuteMsg::AddFee {
        base_fee: Uint128::from(1_000000000000000000u128),
        quote_fee: Uint128::from(9_000000u128),
    };

    let env = inj_mock_env();
    let info = mock_info("addr0000", &[]);
    let _res = execute(deps.as_mut(), env.clone(), info, msg).expect("failed to add fee");

    // Withdraw
    let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender: String::from("addr0001"),
        msg: to_binary(&Cw20HookMsg::Withdraw {}).expect("failed to convert to binary"),
        amount: Uint128::new(90_000000000000u128),
    });

    let env = inj_mock_env();
    let info = mock_info("liquidity0000", &[]);
    let res = execute(deps.as_mut(), env.clone(), info, msg).expect("failed to withdraw");
    let log_withdrawn_share = res.attributes.get(2).expect("no log");
    let log_refund_assets = res.attributes.get(3).expect("no log");
    let msg_burn_liquidity = res.messages.get(0).expect("no message");
    let msg_refund_0 = res.messages.get(1).expect("no message");
    let msg_refund_1 = res.messages.get(2).expect("no message");
    assert_eq!(
        msg_refund_0,
        &SubMsg {
            msg: BankMsg::Send {
                to_address: String::from("addr0001"),
                amount: vec![Coin::new(4_500000000000000000u128, "INJ",)],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
        }
    );
    assert_eq!(
        msg_refund_1,
        &SubMsg {
            msg: BankMsg::Send {
                to_address: String::from("addr0001"),
                amount: vec![Coin::new(40_500000u128, "USDT",)],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
        }
    );
    assert_eq!(
        msg_burn_liquidity,
        &SubMsg {
            msg: WasmMsg::Execute {
                contract_addr: String::from("liquidity0000"),
                msg: to_binary(&Cw20ExecuteMsg::Burn {
                    amount: Uint128::from(90_000000000000u128),
                })
                .expect("failed to convert to binary"),
                funds: vec![],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
        }
    );

    assert_eq!(
        log_withdrawn_share,
        &attr("withdrawn_share", 90_000000000000u128.to_string())
    );
    assert_eq!(
        log_refund_assets,
        &attr("refund_assets", "4500000000000000000INJ, 40500000USDT")
    );

    // Fail to withdraw fee as non owner
    let msg = ExecuteMsg::WithdrawFee {
        base_fee: Uint128::from(1_000000000000000000u128),
        quote_fee: Uint128::from(9_000000u128),
    };

    let env = inj_mock_env();
    let info = mock_info("addr0001", &[]);
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(res, ContractError::Unauthorized {});

    // Fail to withdraw fee more than collected
    let msg = ExecuteMsg::WithdrawFee {
        base_fee: Uint128::from(2_000000000000000000u128),
        quote_fee: Uint128::from(9_000000u128),
    };

    let env = inj_mock_env();
    let info = mock_info("addr0000", &[]);
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(
        res,
        ContractError::CustomError {
            val: String::from("Insufficient fee accrued")
        }
    );

    // Fail to withdraw fee more than collected
    let msg = ExecuteMsg::WithdrawFee {
        base_fee: Uint128::from(1_000000000000000000u128),
        quote_fee: Uint128::from(10_000000u128),
    };

    let env = inj_mock_env();
    let info = mock_info("addr0000", &[]);
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(
        res,
        ContractError::CustomError {
            val: String::from("Insufficient fee accrued")
        }
    );

    // Withdraw fee
    let msg = ExecuteMsg::WithdrawFee {
        base_fee: Uint128::from(1_000000000000000000u128),
        quote_fee: Uint128::from(9_000000u128),
    };

    let env = inj_mock_env();
    let info = mock_info("addr0000", &[]);
    let res = execute(deps.as_mut(), env.clone(), info, msg).expect("failed to withdraw fee");
    let messages = res.messages;
    assert_eq!(
        messages,
        vec![SubMsg {
            msg: BankMsg::Send {
                to_address: String::from("addr0000"),
                amount: vec![
                    Coin::new(1_000000000000000000u128, "INJ",),
                    Coin::new(9_000000u128, "USDT",)
                ],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
        }]
    );
    let attributes = res.attributes;
    assert_eq!(attributes.len(), 1);
    assert_eq!(
        attributes[0],
        &attr("fee_withdrawn", "1000000000000000000INJ, 9000000USDT")
    );
}

#[test]
fn test_swap() {
    let mut deps = test_deps();

    deps.querier.with_token_balances(&[(
        &"liquidity0000".to_string(),
        &[(
            &String::from("addr0001"),
            &Uint128::new(180_000000000000u128),
        )],
    )]);
    deps.querier.with_balance(&[(
        &String::from(TEST_CONTRACT_ADDR),
        &[
            Coin {
                denom: "INJ".to_string(),
                amount: Uint128::from(10_000000000000000000u128),
            },
            Coin {
                denom: "USDT".to_string(),
                amount: Uint128::from(90_000000u128),
            },
        ],
    )]);

    let market_id = MarketId::new(TEST_MARKET_ID.to_string()).expect("failed to create market_id");
    let msg = InstantiateMsg {
        owner: "addr0000".to_string(),
        market_id: market_id.clone(),
        base_decimal: 18,
        quote_decimal: 6,
        base_price_id: "INJ_PRICE_ID".to_string(),
        quote_price_id: "USDT_PRICE_ID".to_string(),
        hardcap: Uint128::new(5000_000000000000u128),
        token_code_id: 10u64,
    };

    let env = inj_mock_env();
    let info = mock_info("addr0000", &[]);
    let _res = instantiate(deps.as_mut(), env.clone(), info, msg).expect("failed to instantiate");

    // Store liquidity token
    store_liquidity_token(deps.as_mut(), 1, "liquidity0000".to_string());

    let sender_addr = "inj1x2ck0ql2ngyxqtw8jteyc0tchwnwxv7npaungt";
    let env = inj_mock_env();
    let info = mock_info(sender_addr, &[]);
    let msg = ExecuteMsg::SwapSpot {
        buying: true,
        quantity: i32_to_dec(8),
        price: i32_to_dec(1000),
    };
    let res = execute(deps.as_mut(), env.clone(), info, msg.clone()).unwrap_err();
    assert_eq!(res, ContractError::Unauthorized {});

    let info = mock_info("addr0000", &[]);
    let res = execute(deps.as_mut(), env.clone(), info, msg.clone())
        .expect("failed to place limit order");

    let subaccount_id = SubaccountId::new(
        "0xade4a5f5803a439835c636395a8d648dee57b2fc000000000000000000000000".to_string(),
    )
    .expect("failed to create subaccount_id");

    let expected_atomic_order_message = BatchUpdateOrders {
        sender: env.contract.address.to_owned(),
        subaccount_id: None,
        spot_market_ids_to_cancel_all: vec![],
        derivative_market_ids_to_cancel_all: vec![],
        spot_orders_to_cancel: vec![],
        derivative_orders_to_cancel: vec![],
        spot_orders_to_create: vec![ShortSpotOrder {
            market_id,
            order_info: ShortOrderInfo {
                subaccount_id: subaccount_id.into(),
                fee_recipient: Some(env.contract.address),
                price: i32_to_dec(1000),
                quantity: i32_to_dec(8),
            },
            order_type: OrderType::Buy,
            trigger_price: None,
        }],
        derivative_orders_to_create: vec![],
    };

    let order_message = get_message_data(&res.messages, 0);
    assert_eq!(
        InjectiveRoute::Exchange,
        order_message.route,
        "route was incorrect"
    );
    assert_eq!(
        expected_atomic_order_message, order_message.msg_data,
        "spot create order had incorrect content"
    );

    let binary_response = Binary::from_base64("CAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBCAEIAQgBEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEQARABEAEaDDB4MTIzNDU2Nzg5MA==").expect("failed to decode message");
    let reply_msg = Reply {
        id: ORDER_REPLY_ID,
        result: SubMsgResult::Ok(SubMsgResponse {
            events: vec![],
            data: Some(binary_response),
        }),
    };

    let transfers_response =
        reply(deps.as_mut(), inj_mock_env(), reply_msg).expect("failed to reply");
    let messages = transfers_response.messages;
    assert_eq!(messages.len(), 0);
    let attributes = transfers_response.attributes;
    assert_eq!(attributes.len(), 1);
    assert_eq!(
        attributes[0],
        &attr("order_hash", "0x1234567890".to_string())
    );
}

fn create_pyth_price_handler() -> impl HandlesPythPriceQuery {
    struct Temp();
    impl HandlesPythPriceQuery for Temp {
        fn handle(&self, price_id: String) -> QuerierResult {
            let start = SystemTime::now();
            let since_the_epoch = start
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards");
            let timestamp = (since_the_epoch.as_secs() as i64) - 30;
            let response = match price_id.as_str() {
                "INJ_PRICE_ID" => PythPriceResponse {
                    price_state: Some(PythPriceState {
                        price_id: "INJ_PRICE_ID".to_string(),
                        ema_price: FPDecimal::from_str("9").expect("failed to parse string"),
                        ema_conf: FPDecimal::from_str("9").expect("failed to parse string"),
                        conf: FPDecimal::from_str("9").expect("failed to parse string"),
                        publish_time: timestamp,
                        price_state: PriceState {
                            price: FPDecimal::from_str("9").expect("failed to parse string"),
                            cumulative_price: FPDecimal::from_str("9")
                                .expect("failed to parse string"),
                            timestamp,
                        },
                    }),
                },
                "USDT_PRICE_ID" => PythPriceResponse {
                    price_state: Some(PythPriceState {
                        price_id: "USDT_PRICE_ID".to_string(),
                        ema_price: FPDecimal::one(),
                        ema_conf: FPDecimal::one(),
                        conf: FPDecimal::one(),
                        publish_time: timestamp,
                        price_state: PriceState {
                            price: FPDecimal::one(),
                            cumulative_price: FPDecimal::one(),
                            timestamp,
                        },
                    }),
                },
                _ => PythPriceResponse {
                    price_state: Some(PythPriceState {
                        price_id:
                            "0xff0ec26442c57d7456695b843694e7379b15cf1b250b27e0e47e657f1955aaff"
                                .to_string(),
                        ema_price: FPDecimal::one(),
                        ema_conf: FPDecimal::one(),
                        conf: FPDecimal::one(),
                        publish_time: timestamp,
                        price_state: PriceState {
                            price: FPDecimal::one(),
                            cumulative_price: FPDecimal::one(),
                            timestamp,
                        },
                    }),
                },
            };
            SystemResult::Ok(ContractResult::from(to_binary(&response)))
        }
    }
    Temp()
}

fn create_spot_market_handler() -> impl HandlesMarketIdQuery {
    struct Temp();
    impl HandlesMarketIdQuery for Temp {
        fn handle(&self, market_id: MarketId) -> QuerierResult {
            if market_id
                == MarketId::new(TEST_MARKET_ID.to_string()).expect("failed to create market_id")
            {
                let response = SpotMarketResponse {
                    market: Some(SpotMarket {
                        ticker: "INJ/USDT".to_string(),
                        base_denom: "INJ".to_string(),
                        quote_denom: "USDT".to_string(),
                        maker_fee_rate: FPDecimal::from_str("0.01")
                            .expect("failed to parse string"),
                        taker_fee_rate: FPDecimal::from_str("0.1").expect("failed to parse string"),
                        relayer_fee_share_rate: FPDecimal::from_str("0.4")
                            .expect("failed to parse string"),
                        market_id,
                        status: MarketStatus::Active,
                        min_price_tick_size: FPDecimal::from_str("0.000000000000001")
                            .expect("failed to parse string"),
                        min_quantity_tick_size: FPDecimal::from_str("1000000000000000")
                            .expect("failed to parse string"),
                    }),
                };
                SystemResult::Ok(ContractResult::from(to_binary(&response)))
            } else {
                let response = SpotMarketResponse { market: None };
                SystemResult::Ok(ContractResult::from(to_binary(&response)))
            }
        }
    }
    Temp()
}
