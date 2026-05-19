# REFERENCES.md

이 문서는 Rust migration 설계 시 확인한 공식/준공식 reference 목록이다. 구현 시점에 다시 확인해야 한다.

## Polymarket

- Clients & SDKs: `https://docs.polymarket.com/api-reference/clients-sdks`
  - TypeScript, Python, Rust official client와 CLOB API 지원 범위를 확인한다.
- Deposit Wallets: `https://docs.polymarket.com/trading/deposit-wallets`
  - Rust SDK의 deposit wallet CLOB order path와 relayer client gap을 확인한다.
- CLOB V2 Migration: `https://docs.polymarket.com/v2-migration`
  - production host, pUSD collateral, V1 compatibility 제거 여부를 확인한다.
- Authentication: `https://docs.polymarket.com/api-reference/authentication`
  - L1/L2 auth, API credentials, order signing requirements.
- Create Order: `https://docs.polymarket.com/trading/orders/create`
  - order creation and limit order semantics.
- Rewards current configurations: `https://docs.polymarket.com/api-reference/rewards/get-current-active-rewards-configurations`
  - reward market discovery.
- Order scoring: `https://docs.polymarket.com/api-reference/trade/get-order-scoring-status`
  - reward scoring status.
- Rate limits: `https://docs.polymarket.com/api-reference/rate-limits`
  - API backoff/retry policy.
- rs-clob-client-v2 GitHub: `https://github.com/Polymarket/rs-clob-client-v2`
  - Rust client feature flags, typed builders, examples, current issues.


## Third-party relayer candidate references

- `OrderBookTrade/rs-builder-relayer-client`
  - Treat as reference/fork starting point only unless audited.
  - Upstream Safe/Proxy-oriented assumptions must be removed or isolated before deposit-wallet live use.
  - Production import requires an organization-controlled internal fork with WALLET/WALLET-CREATE support and pinned revision.

## Rust

- Rust language: `https://www.rust-lang.org/`
  - type/ownership/concurrency safety.
- Rust API Guidelines: `https://rust-lang.github.io/api-guidelines/`
  - public API naming, docs, metadata, consistency.
- Clippy docs: `https://doc.rust-lang.org/clippy/`
  - lint categories and configuration.
- Rust Clippy GitHub: `https://github.com/rust-lang/rust-clippy`
  - lint allow/warn/deny configuration.
- Tokio docs: `https://docs.rs/tokio`
  - async runtime.
- Tokio channel tutorial: `https://tokio.rs/tokio/tutorial/channels`
  - mpsc/oneshot command manager pattern.
- Rust Design Patterns - Newtype: `https://rust-unofficial.github.io/patterns/patterns/behavioural/newtype.html`
  - type safety and encapsulation pattern.
- The Rust Book - OOP state pattern: `https://doc.rust-lang.org/book/ch18-03-oo-design-patterns.html`
  - state pattern concept and why Rust enum/typestate alternatives may be preferable.

## Verification note

외부 문서는 변경될 수 있다. venue-facing change를 하기 전에는 다음 순서로 확인한다.

```text
1. docs.polymarket.com/llms.txt에서 canonical page 확인
2. official docs page 확인
3. rs-clob-client-v2 source/README 확인
4. raw HTTP status/body 확인
5. adapter regression test 작성
6. production logic 변경
```

## Official package names

Use the official Rust package/repository only:

```text
package: polymarket_client_sdk_v2
repository: Polymarket/rs-clob-client-v2
```

Relayer SDK support is official for TypeScript/Python. Rust relayer functionality must be raw REST adapter or sidecar unless official support appears and is reviewed.

## Forked relayer references

- Official Relayer SDK list: `https://docs.polymarket.com/api-reference/clients-sdks`
  - Relayer SDK is listed for TypeScript and Python; Rust relayer support must be raw REST, sidecar, or reviewed internal fork.
- Deposit Wallet guide: `https://docs.polymarket.com/trading/deposit-wallets`
  - Rust supports deposit-wallet CLOB order path but not builder relayer client; WALLET-CREATE and WALLET are relayer/raw API flows.
- Gasless transactions: `https://docs.polymarket.com/trading/gasless`
  - relayer auth headers, transaction states, pUSD approvals, CTF operations.
- Third-party reference only: `https://github.com/OrderBookTrade/rs-builder-relayer-client`
  - can be read/forked as reference; do not import directly for production deposit-wallet flow.
