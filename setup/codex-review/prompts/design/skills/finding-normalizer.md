# Skill: Finding Normalizer

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
finding을 invariant 중심 design item으로 바꾸는 세부 지침.

## 입력 컨텍스트
- design prompt body
- design artifact

## 출력 계약
상위 prompt에 포함될 reusable instruction block.

## 반드시 지킬 규칙
- 역할을 벗어난 결정을 하지 않는다.
- 출력 schema field 이름을 바꾸지 않는다.
- 불확실성은 evidence, acceptance criteria, required tests로 좁힌다. design_chief needs_human 근거는 non-executable blocker에만 한정한다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
