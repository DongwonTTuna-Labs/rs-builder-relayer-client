# AGENTS.md

사용자 언어: 한국어

## Operating Contract

- PR은 절대 직접 머지하지 말 것.
- private key, API key, bot token, OAuth secret 같은 비밀값은 로그, 응답, 커밋에 노출하지 말 것.
- 이 repo는 Forgejo Actions를 사용한다. CI와 Codex review workflow는 `.forgejo/workflows`에 두고, workflow helper는 `.forgejo/scripts`에 둔다.
- 이전 CI 메타데이터 디렉터리를 다시 추가하지 않는다.
- 이 fork는 공식 Polymarket Rust relayer SDK가 아니다. venue-facing 동작은 공식 문서, Python/TypeScript relayer SDK, 로컬 테스트 벡터를 먼저 대조한 뒤 구현한다.
- branch dependency는 production에서 사용하지 않는다. consumer repo는 commit SHA pin 또는 path dependency만 사용한다.
- `WALLET-CREATE`, `WALLET`, EIP-712 DepositWallet Batch, nonce, polling, pUSD/CTF adapter calldata를 구현하기 전까지 deposit-wallet live execution 가능하다고 말하지 않는다.
- relayer API key identity, owner signer, deposit wallet/funder address는 같은 값으로 가정하지 않는다.
- 이 repo는 consumer app에 import되는 모듈이다. exported/public API 변경은 항상 보수적으로 다루고, 기존 public type/function signature를 바꾸거나 제거하지 않는다.
- 이 문서의 모든 rule은 기본적으로 hard rule이다. 단순 편의, 빠른 구현, lint/format 우회, 임시 scaffold, 추정 기반 wire format 변경을 이유로 rule을 완화하지 않는다. 정말 대안이 없거나, 해당 rule을 지키면 구현 자체가 불가능하다는 원초적인 한계가 있을 때만 예외를 검토한다.
- rule 예외가 필요하면 같은 PR 안에서 근거, 실패한 대안, 원초적 한계, consumer 영향도, migration/rollback path를 먼저 문서화한다. 이 증거 없이 rule을 깨는 변경을 넣지 않는다.

## Strict Engineering Rules

### Failure Handling Contract

- 문제가 발생하면 증상 완화보다 먼저 근본 원인을 추적한다.
- root cause 기록에는 최소한 `who`, `what`, `when`, `why`, `how`를 남긴다. `who`는 개인 탓이 아니라 agent, script, command, dependency, API, config 같은 실행 주체를 뜻한다.
- 재시도, sleep, fallback, ignore, allow, unwrap 대체 같은 우회는 root cause가 식별되기 전에는 넣지 않는다.
- 동일 failure가 반복되면 세 번째 시도 전에 원인 분류와 회귀 테스트 또는 fixture를 먼저 추가한다.

### No Workaround First

- senior engineer가 유지보수할 수 있는 근본 해결을 먼저 설계한다.
- workaround는 `temporary`, `bounded`, `removal condition`, `owner`, `risk`가 문서화될 때만 허용한다.
- "일단 통과시키기 위한" lint allow, test 삭제, broad mock, API shape 추정, silent fallback은 금지한다.

### Evidence And Claim Rules

- "된다", "고쳤다", "안전하다", "호환된다"는 말은 검증 증거 없이는 쓰지 않는다.
- public API, wire format, signing, nonce, auth, calldata 변경은 fixture/golden test 또는 공식 SDK 대조 증거가 있어야 한다.
- PR 설명에는 중요한 검증 증거와 남은 리스크를 명확히 남긴다.

### Change Scope

- setup PR, docs PR, behavior PR, live-capable PR을 섞지 않는다.
- live/order-capable logic은 이름, 문서, placeholder, adapter wiring PR과 분리한다.
- 기존 upstream Safe/Proxy 동작은 deposit-wallet 구현을 위해 암묵적으로 바꾸지 않는다.

### External API And Wire Format

- Polymarket 문서, official TS/Python SDK, 실제 HTTP request/response 중 최소 하나와 대조 없이 wire format을 만들지 않는다.
- 문서와 SDK가 다르면 차이를 문서화하고, 어느 쪽을 source of truth로 삼는지 PR에 남긴다.
- unknown transaction state, ambiguous nonce, partial submit response는 성공으로 간주하지 않는다.

### Security And Dependency

- secret-bearing type은 `Debug`, error, log, snapshot, fixture에 원문 노출되지 않게 설계한다.
- signer identity, relayer auth identity, deposit wallet/funder는 타입/API 레벨에서 분리한다.
- private key/API key가 필요한 테스트는 기본 test suite에 넣지 않고 explicit manual gate로 분리한다.
- production dependency는 branch pin 금지, commit SHA pin만 허용한다.
- unofficial crate/fork는 public API surface와 accepted risk를 문서화한 뒤 consumer에 연결한다.
- dependency bump는 lockfile 변경만 보지 말고 public API, transitive crypto/signing crate, reqwest/tls 영향을 확인한다.

### Testing And Rollback

- fixture test 없는 signing/calldata/serialization 변경은 금지한다.
- mock은 unit boundary에서만 쓰고, production path에 fake venue client를 만들지 않는다.
- flaky test를 완화하기 전에 왜 flaky한지 원인을 기록한다.
- breaking change가 아니더라도 consumer 영향이 있으면 migration path를 남긴다.
- rollback이 불가능한 변경은 그 이유와 운영 중단 기준을 PR에 적는다.
- stateful/live-capable 변경은 enable flag, dry-run evidence, rollback path 없이는 merge 대상이 아니다.

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
5. 기존 rule을 완화하거나 예외 처리하려면 구현 전에 근거와 대안 검토를 문서화한다.

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
