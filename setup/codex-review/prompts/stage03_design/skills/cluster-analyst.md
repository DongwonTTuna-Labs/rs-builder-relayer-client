# Skill: Cluster Analyst

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
cluster별 설계 분석 깊이와 evidence 기준.

## 입력 컨텍스트
- stage03 prompt body
- stage03 artifact

## 출력 계약
상위 prompt에 포함될 reusable instruction block.

## 반드시 지킬 규칙
- 역할을 벗어난 결정을 하지 않는다.
- 출력 schema field 이름을 바꾸지 않는다.
- 불확실성은 evidence, acceptance criteria, required tests로 좁힌다. stage04 needs_human 근거는 non-executable blocker에만 한정한다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
