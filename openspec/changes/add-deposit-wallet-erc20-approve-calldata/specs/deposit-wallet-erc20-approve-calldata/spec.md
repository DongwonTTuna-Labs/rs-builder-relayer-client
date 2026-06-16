## ADDED Requirements

### Requirement: ERC20 approve calldata builder
The module path `src/deposit_wallet/calldata.rs::build_erc20_approve_call` MUST expose the canonical signature `build_erc20_approve_call(token: Address, spender: Address, amount: U256) -> Result<DepositWalletCall, RelayerError>`.

Given a token `Address`, spender `Address`, and `U256` amount, the builder MUST return `Ok(DepositWalletCall { target = token, value = 0, data = 0x095ea7b3 ++ abi_encode(spender, amount) })` for valid nonzero addresses. It MUST use the canonical ERC20 `approve(address,uint256)` selector `0x095ea7b3` and MUST encode arguments in the exact order `(spender, amount)`.

The builder MUST reject the zero address for `token` or `spender` with `Err(RelayerError...)`, and MUST NOT panic. Address well-formedness is the caller's responsibility because inputs are already typed `Address`; the builder MUST NOT parse malformed address strings. Zero-address rejection is an EXPERIMENT-SCOPED validation chosen for a deterministic typed-input-reachable `Err` path. Standard ERC20 permits a zero spender as an allowance revoke, so this default is overridable by a later spec.

The returned `DepositWalletCall` MUST compose into a deposit-wallet `WALLET` batch as one element of `calls: Vec<DepositWalletCall>` while the existing 256-call and 1 MiB batch limits remain in force.

#### Scenario: Valid inputs build exact approve calldata
- **Given** a nonzero token `Address`, a nonzero spender `Address`, and a `U256` amount
- **When** `src/deposit_wallet/calldata.rs::build_erc20_approve_call(token, spender, amount)` is called
- **Then** the result MUST be `Ok(DepositWalletCall { target = token, value = 0, data = 0x095ea7b3 ++ abi_encode(spender, amount) })`
- **And** the `data` bytes MUST start with selector `0x095ea7b3` followed by ABI-encoded `(spender, amount)`

#### Scenario: Amount edge values encode as uint256 words
- **Given** a nonzero token `Address`, a nonzero spender `Address`, and amount inputs `0` and `U256::MAX`
- **When** `build_erc20_approve_call(token, spender, amount)` is called for each amount
- **Then** both calls MUST return `Ok(DepositWalletCall)` with `target == token` and `value == 0`
- **And** each `data` field MUST encode the amount as the correct 32-byte big-endian ABI `uint256` word after selector `0x095ea7b3` and the spender word

#### Scenario: Zero token or spender returns an error without panic
- **Given** a zero-address `token` with a nonzero spender, or a nonzero token with a zero-address `spender`
- **When** `build_erc20_approve_call(token, spender, amount)` is called
- **Then** the builder MUST return `Err(RelayerError...)`
- **And** the builder MUST NOT panic for either zero-address input

#### Scenario: Builder output composes into WALLET batch calls
- **Given** a successful approve call output and a deposit-wallet `WALLET` batch `calls: Vec<DepositWalletCall>` that remains within 256 calls and 1 MiB total calldata
- **When** the approve output is inserted as one batch call
- **Then** the inserted call MUST preserve `target == token`, `value == 0`, and `data == 0x095ea7b3 ++ abi_encode(spender, amount)`
- **And** the builder MUST NOT require signing, nonce lookup, HTTP submit, transaction polling, wallet deployment, or production address selection to create that call
