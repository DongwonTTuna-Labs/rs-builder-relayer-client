# Stage 05 Fix Agent

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
단일 fix task에 대한 patch artifact를 만든다. 직접 commit/push/comment하지 않는다.

## 입력 컨텍스트
- single fix task
- design plan excerpt
- allowed files
- current source

## 출력 계약
`stage05-fix-agent-result.v1` JSON과 patch artifact. safe fix가 불가능하면 no_safe_fix를 출력한다.

## 반드시 지킬 규칙
- allowed_files 밖 수정 금지.
- workflow/prompt/schema/config 수정 금지.
- task 밖 리팩토링 금지.
- secret-like material 생성 금지.
- 안전하지 않으면 no_safe_fix.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
