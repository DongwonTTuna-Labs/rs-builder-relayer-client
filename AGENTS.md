# AGENTS.md

사용자 언어: 한국어

## Operating Contract

- PR은 절대 직접 머지하지 말 것.
- private key, API key, bot token, OAuth secret 같은 비밀값은 로그, 응답, 커밋에 노출하지 말 것.
- 이 fork는 공식 Polymarket Rust relayer SDK가 아니다. venue-facing 동작은 공식 문서, Python/TypeScript relayer SDK, 로컬 테스트 벡터를 먼저 대조한 뒤 구현한다.
- branch dependency는 production에서 사용하지 않는다. consumer repo는 commit SHA pin 또는 path dependency만 사용한다.
- `WALLET-CREATE`, `WALLET`, EIP-712 DepositWallet Batch, nonce, polling, pUSD/CTF adapter calldata를 구현하기 전까지 deposit-wallet live execution 가능하다고 말하지 않는다.
- relayer API key identity, owner signer, deposit wallet/funder address는 같은 값으로 가정하지 않는다.
- 이 repo는 consumer app에 import되는 모듈이다. exported/public API 변경은 항상 보수적으로 다루고, 기존 public type/function signature를 바꾸거나 제거하지 않는다. breaking change가 필요하면 별도 PR에서 근거, migration path, consumer 영향도를 먼저 문서화한다.

## Documentation Map

- `docs/FORK_SCOPE.md`: fork의 범위와 non-goal.
- `docs/FORKED_RELAYER_CRATE.md`: fork 사용 정책, required API, production approval gate.
- `docs/DEPOSIT_WALLET_RELAYER_DESIGN.md`: `WALLET-CREATE`, `WALLET`, nonce, EIP-712, transaction polling 설계.
- `docs/SECURITY.md`: secret, signing, identity separation, supply-chain policy.
- `docs/TESTING.md`: fork에서 반드시 통과해야 하는 unit/golden/manual gate.
- `docs/DECISIONS.md`: relayer fork 관련 ADR.
- `docs/CONSUMER_INTEGRATION.md`: consumer Rust app에서 이 crate를 import하는 방식.
- `docs/REVIEW_CHECKLIST.md`: PR/release review checklist.
- `docs/REFERENCES.md`: 공식 문서와 reference repo 목록.
- `docs/LEGACY_SAFE_PROXY_RELAYER_GUIDE.md`: upstream Safe/Proxy guide. deposit-wallet 구현 기준으로 사용하지 않는다.
- `docs/PUBLISHING_DISABLED.md`: crates.io publish 금지와 git rev pin 정책.

## Scope

- 기존 upstream Safe/Proxy relayer 코드는 audit reference로 보존한다.
- deposit wallet 전용 구현은 `src/deposit_wallet/` 아래에 둔다.
- CLOB order/sign/cancel은 이 crate의 책임이 아니다. consumer repo에서 공식 Rust CLOB SDK를 사용한다.
- live/order-capable 예제는 명시적인 dry-run 기본값과 별도 승인 없이 추가하지 않는다.

## Before Editing

1. `docs/FORKED_RELAYER_CRATE.md`와 `docs/DEPOSIT_WALLET_RELAYER_DESIGN.md`를 먼저 확인한다.
2. venue-facing wire format이면 Polymarket 공식 문서 또는 official Python/TypeScript SDK와 대조한다.
3. signing, nonce, auth identity, calldata를 바꾸면 golden test를 먼저 추가하거나 같은 PR에 포함한다.
4. upstream Safe/Proxy helper를 deposit-wallet flow에 재사용하려면 request body, nonce source, signer identity가 같은지 증명한다.

## Required Validation

최소 확인:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
git diff --check
```

deposit-wallet 기능 PR은 추가로 다음 증거가 필요하다:

```text
WALLET-CREATE request serialization fixture
GET /nonce?type=WALLET request fixture
DepositWallet Batch EIP-712 digest/signature fixture
WALLET submit request serialization fixture
transaction state parser fixture
pUSD/CTF adapter calldata fixture
auth identity != owner signer test
```
