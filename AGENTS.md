# AGENTS.md

사용자 언어: 한국어

## Operating Contract

- PR은 절대 직접 머지하지 말 것.
- private key, API key, bot token, OAuth secret 같은 비밀값은 로그, 응답, 커밋에 노출하지 말 것.
- 이 fork는 공식 Polymarket Rust relayer SDK가 아니다. venue-facing 동작은 공식 문서, Python/TypeScript relayer SDK, 로컬 테스트 벡터를 먼저 대조한 뒤 구현한다.
- branch dependency는 production에서 사용하지 않는다. consumer repo는 commit SHA pin 또는 path dependency만 사용한다.
- `WALLET-CREATE`, `WALLET`, EIP-712 DepositWallet Batch, nonce, polling, pUSD/CTF adapter calldata를 구현하기 전까지 deposit-wallet live execution 가능하다고 말하지 않는다.
- relayer API key identity, owner signer, deposit wallet/funder address는 같은 값으로 가정하지 않는다.

## Scope

- 기존 upstream Safe/Proxy relayer 코드는 audit reference로 보존한다.
- deposit wallet 전용 구현은 `src/deposit_wallet/` 아래에 둔다.
- CLOB order/sign/cancel은 이 crate의 책임이 아니다. consumer repo에서 공식 Rust CLOB SDK를 사용한다.
- live/order-capable 예제는 명시적인 dry-run 기본값과 별도 승인 없이 추가하지 않는다.
