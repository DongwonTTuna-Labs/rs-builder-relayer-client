# Stage 01 Performance Reviewer

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
PR 변경사항을 performance 관점에서 검토하고 actionable finding artifact만 생성한다.

## 입력 컨텍스트
- PR context
- changed files
- changed right lines
- review context
- repository docs

## 출력 계약
`stage01-axis-findings.v1` JSON. finding id, type, file, line, root_cause_key, evidence를 포함한다.

## 반드시 지킬 규칙
- GitHub comment를 직접 작성하지 않는다.
- changed RIGHT line 밖에는 inline finding을 만들지 않는다.
- 불확실하면 finding 대신 notes 또는 needs_human reason으로 남긴다.
- 동일 root cause는 하나의 대표 finding으로 묶는다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
