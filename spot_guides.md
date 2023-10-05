### 1. Setup

Follow [this](https://docs.injective.network/develop/guides/cosmwasm-dapps/Cosmwasm_deployment_guide_Testnet/#2-download-dockerised-injective-chain-binary) step to setup `injectived`.

### 2. Deposit

Execute below commands to deposit into the vault.

```bash
export INJ_ADDRESS=YOUR_INJ_ADDRESS
export CONTRACT=inj13c6dmrsmp26tqk5gxhksp89ze8kv6mnm5nhzm0
export DEPOSIT='{"deposit":{"assets":[{"info":{"denom":"inj"},"amount":"1000000000000000000"},{"info":{"denom":"peggy0x87aB3B4C8661e07D6372361211B96ed4Dc36B1B5"},"amount":"8000000"}],"receiver":"YOUR_INJ_ADDRESS_OR_OTHER_ADDRESS"}}'
yes 12345678 | injectived tx wasm execute $CONTRACT "$DEPOSIT" --from=$(echo $INJ_ADDRESS) --chain-id="injective-888" --yes --gas-prices=500000000inj --gas=20000000 --node=https://k8s.testnet.tm.injective.network:443 --amount=1000000000000000000inj,8000000peggy0x87aB3B4C8661e07D6372361211B96ed4Dc36B1B5
```

### 3. Withdraw

Simply send the vault LP token to the vault to withdraw funds

### 4. SwapSpot (for owner only)

Execute below commands to place limit order.

```bash
export INJ_ADDRESS=YOUR_INJ_ADDRESS
export CONTRACT=inj13c6dmrsmp26tqk5gxhksp89ze8kv6mnm5nhzm0
export SWAP_SPOT='{"swap_spot":{"buying":true,"quantity":"1","price":"7.5"}}'
yes 12345678 | injectived tx wasm execute $CONTRACT "$SWAP_SPOT" --from=$(echo $INJ_ADDRESS) --chain-id="injective-888" --yes --gas-prices=500000000inj --gas=20000000 --node=https://k8s.testnet.tm.injective.network:443
```

### 5. CancelOrder (for owner only)

Execute below commands to cancel limit order.

```bash
export INJ_ADDRESS=YOUR_INJ_ADDRESS
export CONTRACT=inj13c6dmrsmp26tqk5gxhksp89ze8kv6mnm5nhzm0
export CANCEL_ORDER='{"cancel_order":{"order_hash":"ORDER_HASH_HERE"}}'
yes 12345678 | injectived tx wasm execute $CONTRACT "$CANCEL_ORDER" --from=$(echo $INJ_ADDRESS) --chain-id="injective-888" --yes --gas-prices=500000000inj --gas=20000000 --node=https://k8s.testnet.tm.injective.network:443
```

### 6. AddFee (for owner only)

Execute below commands to cancel limit order.

```bash
export INJ_ADDRESS=YOUR_INJ_ADDRESS
export CONTRACT=inj13c6dmrsmp26tqk5gxhksp89ze8kv6mnm5nhzm0
export CANCEL_ORDER='{"add_fee":{"base_fee":"1000000000000000000","quote_fee":"9000000"}}'
yes 12345678 | injectived tx wasm execute $CONTRACT "$CANCEL_ORDER" --from=$(echo $INJ_ADDRESS) --chain-id="injective-888" --yes --gas-prices=500000000inj --gas=20000000 --node=https://k8s.testnet.tm.injective.network:443
```

### 7. WithdrawFee (for owner only)

Execute below commands to cancel limit order.

```bash
export INJ_ADDRESS=YOUR_INJ_ADDRESS
export CONTRACT=inj13c6dmrsmp26tqk5gxhksp89ze8kv6mnm5nhzm0
export CANCEL_ORDER='{"withdraw_fee":{"base_fee":"1000000000000000000","quote_fee":"9000000"}}'
yes 12345678 | injectived tx wasm execute $CONTRACT "$CANCEL_ORDER" --from=$(echo $INJ_ADDRESS) --chain-id="injective-888" --yes --gas-prices=500000000inj --gas=20000000 --node=https://k8s.testnet.tm.injective.network:443
```

### 8. Query Owner

Execute below commands to query contract owner.

```bash
export CONTRACT=inj13c6dmrsmp26tqk5gxhksp89ze8kv6mnm5nhzm0
export OWNER_QUERY='{"owner":{}}'
injectived query wasm contract-state smart $CONTRACT "$OWNER_QUERY" --node=https://k8s.testnet.tm.injective.network:443
```

### 9. Query Tokens For Shares

Execute below commands to query tokens for shares.

```bash
export CONTRACT=inj13c6dmrsmp26tqk5gxhksp89ze8kv6mnm5nhzm0
export TOKENS_FOR_SHARES_QUERY='{"tokens_for_shares":{"share":"1000000000000000000"}}'
injectived query wasm contract-state smart $CONTRACT "$TOKENS_FOR_SHARES_QUERY" --node=https://k8s.testnet.tm.injective.network:443
```

### 10. Query Total Liquidity

Execute below commands to query total liquidity.

```bash
export CONTRACT=inj13c6dmrsmp26tqk5gxhksp89ze8kv6mnm5nhzm0
export TOTAL_LIQUIDITY_QUERY='{"total_liquidity":{}}'
injectived query wasm contract-state smart $CONTRACT "$TOTAL_LIQUIDITY_QUERY" --node=https://k8s.testnet.tm.injective.network:443
```

### 11. Query Prices

Execute below commands to query token prices.

```bash
export CONTRACT=inj13c6dmrsmp26tqk5gxhksp89ze8kv6mnm5nhzm0
export PRICES_QUERY='{"prices":{}}'
injectived query wasm contract-state smart $CONTRACT "$PRICES_QUERY" --node=https://k8s.testnet.tm.injective.network:443
```
