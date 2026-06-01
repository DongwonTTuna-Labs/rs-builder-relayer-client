# Safety Rules

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
보안/권한/secret/side effect 금지 규칙을 정의한다.

## 입력 컨텍스트
- artifact bundle
- token availability
- repo policy

## 출력 계약
위험한 경우 needs_human 또는 no_safe_fix를 출력한다.

## 반드시 지킬 규칙
- secret-like material 출력 금지.
- model job에서 GitHub write 금지.
- public API/signing/auth/nonce/live-capable 위험 변경은 자동 fix 금지.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
