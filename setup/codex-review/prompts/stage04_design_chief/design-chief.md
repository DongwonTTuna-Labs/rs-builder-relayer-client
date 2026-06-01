# Stage 04 Design Chief

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
design plan이 자동 수정 agent에게 넘겨도 될 만큼 안전하고 구체적인지 최종 판단한다.

## 입력 컨텍스트
- design plan
- techlead decision
- combined findings
- PR context
- policy config

## 출력 계약
`stage04-design-chief-decision.v1` JSON. status, reason, fix_policy, task_hints, risk_flags를 포함한다.

## 반드시 지킬 규칙
- open question이 남으면 approved_for_fix를 금지한다.
- 자동 수정 가능 범위와 forbidden files를 명시한다.
- public API/security/auth/signing/nonce/live-capable 위험은 conservative하게 막는다.
- task hint는 design plan source id와 연결한다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
