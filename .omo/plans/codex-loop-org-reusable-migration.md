# Org-Reusable Codex Loop Migration (Label → repository_dispatch self-driving loop)

## TL;DR

> **Quick Summary**: Port the full sophisticated `setup/codex-review` pipeline (~12,333 LOC) INTO
> `home-server-infra`'s reusable Codex loop core, re-architecting it from a 4-workflow LABEL-driven
> state machine into ONE parameterized `workflow_call` reusable workflow that self-drives via
> `repository_dispatch` until LGTM — no labels/comments as loop state. Then convert
> `rs-builder-relayer-client` into a thin, SHA-pinned consumer and remove its 4 workflows +
> `setup/codex-review/` + label operations.
>
> **Deliverables**:
> - `home-server-infra`: a full-featured, org-reusable Codex loop core (review/design/fix/issue stages,
>   internal matrix fan-out, trusted fix-push continuation, dispatch ledger + loop caps, terminal taxonomy,
>   label/comment-free state via typed payload + explicit state-pointer artifacts).
> - `rs-builder-relayer-client`: thin SHA-pinned consumer adapters (pull_request_target + manual + dispatch),
>   dry-run-first; old label workflows / helper / label ops removed after parity proof.
> - Contract tests, parity tests, negative runtime tests, dry-run dispatch smoke evidence.
>
> **Estimated Effort**: XL (Oracle: 3+ days; cross-repo behavior + security migration)
> **Parallel Execution**: YES — 7 waves (waves gate on PR merge; parallel WITHIN each wave)
> **Critical Path**: 1 → (2..6) → (7..11) → 12 → (13,14,15) → 17 → (20,21) → 22 → 23 → 27 → 29 → (30,31,32) → F1–F4 → user okay

---

## Context

### Original Request
home-server-infra PR #18(`ci(actions): add reusable Codex loop workflows`)에서 만든 reusable Codex loop를
이 repo에 적용해, 기존 워크플로우들을 이걸 사용하도록 "아주 깔끔하게" 대체하고 싶다.

### The WHY (사용자 원본 동기 — 가장 중요)
- 루프는 **LGTM 받을 때까지 자동완주(self-driving)** 해야 한다.
- 현재 라벨/코멘트로 상태를 표시하는 것이 **"너무 드러워서"** 싫다 → 제거 대상.
- 각 단계 끝에서 **커스텀 이벤트(`repository_dispatch`)로 다음 단계를 "소환"** = 자가구동.
  (GitHub 무료 org에서 라벨/코멘트 없이 워크플로우를 체이닝하는 방법.)
- 이 워크플로우를 **다른 repo로 그대로 들고가거나 범용 action처럼** 재사용 가능해야 한다 (org-reusable).

### Interview Summary (confirmed decisions)
- **Q1**: PR #18을 home-server-infra `main`에 **먼저 머지** → 그 commit SHA를 기준으로 **새 PR에서 확장**.
- **Q2**: **fix-push 연속(continuation)까지 이번에 구현** — trusted same-repo push + `updated_head_sha` + redispatch.
- **Q3**: 진입/연속 메커니즘 = **`repository_dispatch` 자가구동 루프** (라벨 전면 제거), LGTM=terminal.
  범용 재사용은 reusable workflow(`workflow_call`) + dispatch adapter 형태 유지(외부 배포는 non-goal).

### Research Findings
- **현재 repo**: 4개 라벨 기반 워크플로우(`codex-review/design/fix/issue.yml`), `pull_request_target` +
  `labeled` 트리거, GitHub App 토큰으로 다음 워크플로우 재트리거, OIDC relay로 Codex 호출, 단계 간 상태를
  **artifact + 라벨 + cross-run `gh run list --headSha` 탐색**으로 전달.
- **로직 본체**: `setup/codex-review/` = Python ~12,333 LOC. subpackages: review(멀티축 5개), resolve_gate,
  techlead, design(inventory/cluster/analyze/plan), design_chief, fix_dispatch, fix_merge, push, issue_fallback.
  30+ JSON schema, 단계별 prompt. 런타임에 composite action이 `${{github.repository}}@${{github.workflow_sha}}`를
  `workflow-helper/`로 checkout + pip install.
- **PR #18 (OPEN, `work/codex-loop-reusable-workflow`)**: reusable core는 **얇은 스캐폴드**.
  validate → trust-and-stale-guard → setup-relay → run-stage(dry-run=placeholder, live=단일 generic 프롬프트) →
  finalize. fix-push 연속은 의도적으로 `trusted-fix-push-not-implemented`로 종료. dispatch/manual 어댑터 + 계약문서 + 테스트 포함.
- **home-server-infra**: 로컬 클론 존재(`/Users/dongwon/Documents/Programming/DongwonTTuna-Labs/home-server-infra/`).
  `main`에는 아직 `.github/` 없음(PR #18 파일은 PR 브랜치에만).

### Metis Review (gaps addressed in this plan)
- 라벨(보이는 상태) → 이벤트(보이지 않는 상태) 전환의 핵심 리스크 = **관찰가능성/종료가시화/루프 캡**. 1급 시민으로 인코딩.
- dispatch 폭주/runaway: iteration cap + per-correlation dispatch ledger + concurrency group + stale-head reject.
- cross-run artifact 회귀: `gh run list --headSha` 탐색 제거, 명시적 state pointer 사용.
- fork/secret 노출, App 토큰 만료(스테이지별 mint), openai/codex-action parity(단일 generic 프롬프트 ≠ rich 파이프라인).

### Oracle Review (`CHECK [5/5] PASS | VERDICT: GO`, design amendments incorporated)
- 신규 발견: `setup/codex-review/src/codex_review/loop/state.py`가 **sticky 코멘트로 루프 메모리 저장** → 제거/legacy화 필수.
- `setup/codex-review/config.yml`에 repo-고정 경로/금지파일 → consumer override 가능하게 추출.
- PR #18 reusable core: iteration `== max` 허용(off-by-one) → `>= max` 거부로 수정. core/dispatch concurrency group 불일치 통일.
- 필수 설계 보강: dispatch payload에 **`schema_version`, `state_run_id`, `state_artifact_name`** 타입드 포인터 추가.
- 체크아웃 토폴로지 4분리(trusted-core / target-base / pr-head(data-only) / pr-head-write(fix push only)).

---

## Work Objectives

### Core Objective
`home-server-infra`에 **풀 기능 org-reusable Codex loop core**를 만들고(정교한 로직 이식 + label/comment-free
self-driving `repository_dispatch` 루프 + trusted fix-push 연속), `rs-builder-relayer-client`를 그 core의
**thin SHA-pinned consumer**로 전환한다(기존 4 워크플로우/helper/라벨 제거).

### Concrete Deliverables
- `home-server-infra/.github/workflows/codex-loop-reusable.yml` (full stage graphs, internal matrix fan-out)
- `home-server-infra/.github/workflows/codex-loop-dispatch.yml`, `codex-loop-manual.yml` (adapters)
- `home-server-infra/setup/codex-review/` (이식된 Python 패키지, consumer-overridable config)
- `home-server-infra/docs/codex-loop-reusable.md` (payload schema v2, checkout topology, terminal taxonomy, rollout/rollback)
- `home-server-infra/tests/workflows/` 계약 테스트 + 이식된 pytest + parity/negative 테스트
- `rs-builder-relayer-client/.github/workflows/`: thin adapters (SHA-pinned) → 이후 4 워크플로우 제거
- `.omo/evidence/`: actionlint/pytest/dry-run dispatch smoke/parity/negative 증거

### Definition of Done
- [ ] `actionlint` 두 repo 워크플로우 전부 0 error
- [ ] `pytest`(이식된 codex_review + workflow contract + parity + negative) 전부 통과
- [ ] dry-run `repository_dispatch` smoke가 HTTP 204 + 결정론적 stage 결과 artifact 생성(증거)
- [ ] live same-repo 루프 1회가 review→…→fix-push→redispatch→LGTM 자동완주 증명(증거) 후 구식 워크플로우 제거
- [ ] `rs-builder-relayer-client`에 라벨 기반 워크플로우/`setup/codex-review`/라벨 ops 잔존 0

### Must Have
- 루프 상태는 **타입드 payload + 명시적 state-pointer artifact + job outputs/summary**로만. 라벨/코멘트 상태 금지.
- consumer는 **commit SHA pin** (AGENTS.md: branch pin 금지).
- 모든 신규 경로 **dry-run-first**; live write는 명시적 게이트 + dry-run 증거 후.
- trusted-core(home-server-infra@SHA)에서만 CLI 실행; **PR head는 데이터로만**(secrets 하에 PR 스크립트 실행 금지).
- fix-push: same-repo + stale-head 재검증 후에만, App installation token으로.
- 모든 비-LGTM 종료는 `terminal_reason` + artifact + job summary + workflow conclusion으로 가시화.

### Must NOT Have (Guardrails)
- 라벨/PR 코멘트/이슈를 **루프 상태**로 사용 금지(코멘트 기반 `loop/state.py` 메모리 제거).
- PAT 사용/문서화/fallback 금지. fork PR fix-push 금지. fork PR에 secrets 노출 금지.
- consumer PR head 코드/스크립트를 secrets 하에 실행/설치 금지.
- `gh run list --headSha`로 cross-run artifact 탐색 금지(명시적 `state_run_id` 사용).
- 마이그레이션 중 prompt/schema/리뷰 로직 "개선" 금지(parity 목적 외). relayer/Rust 소스 변경 금지.
- 외부 배포(marketplace) 스토리 추가 금지. 라벨/체크/코멘트를 다른 이름의 상태로 재도입 금지.
- 구식 워크플로우는 **dry-run + 최소 1회 live 동일-repo 루프 parity 증명 전까지 제거 금지**.

### Spec Framework Integration
- **Detected Framework**: OpenSpec (both repos have `openspec/`). 단, 이 작업은 CI 인프라 변경으로 기존 spec
  요구사항과 직접 매핑되지 않음. 새 spec이 필요하면 `/opsx:propose`로 별도 생성 권장(선택).
- 본 플랜은 spec 산출물을 강제하지 않으며, home-server-infra 쪽 워크플로우 계약 문서(`docs/codex-loop-reusable.md`)가
  사실상의 contract 역할을 한다.

---

## Verification Strategy (MANDATORY)

> **ZERO HUMAN INTERVENTION** — 검증은 전부 에이전트 실행. 단, PR 머지는 AGENTS.md상 maintainer 게이트(아래 명시).

### Test Decision
- **Infrastructure exists**: YES — `setup/codex-review/tests`(pytest), `.github/actionlint.yaml`, PR#18 `tests/workflows/`.
- **Automated tests**: Tests-after + contract-first 혼합. 기존 pytest 보존·이식, 워크플로우 계약 테스트 확장,
  단계별 parity 테스트 + negative runtime 테스트 추가.
- **Framework**: pytest(uvx/py), actionlint, `gh api` dispatch smoke.

### QA Policy
모든 태스크는 에이전트 실행 QA 시나리오 포함. 증거는 `.omo/evidence/task-{N}-{slug}.{ext}`.
- **Workflow YAML**: `actionlint` + workflow-shape pytest (Bash).
- **Python CLI**: `pytest`(이식 경로) + parity fixtures (Bash).
- **Dispatch/loop**: `gh api .../dispatches` dry-run smoke + `gh run view` 결과 파싱 (Bash).
- **Negative**: stale-head/max-iter/fork/closed-PR/missing-app-creds 종료 경로 (Bash, dry-run-safe fixtures).

### Human Gate (AGENTS.md)
- "PR은 절대 직접 머지하지 말 것" → 에이전트는 PR을 **머지하지 않는다**. PR 준비/근거/증거까지만 수행하고
  머지는 maintainer가 한다. Wave 경계(PR 머지)는 maintainer 승인 후 다음 Wave 진행.

---

## Execution Strategy

### Parallel Execution Waves

> Wave는 1개 PR 그룹(AGENTS.md: setup/docs/behavior/live PR 분리)에 대응하고 PR 머지(maintainer 게이트) 후
> 다음 Wave로 진행한다. **Wave 내부에도 선행(prereq) 의존이 있으면 sub-wave(a/b/c…)로 순차** 실행하고, 같은
> sub-wave 안의 태스크만 병렬이다. `[seq]`=직렬 선행, `[‖]`=병렬 그룹. 실제 최대 동시성은 ~4 (Wave 1a/3b).

```
Wave 0 — Prerequisite (human-gated merge):
└── 1. PR #18 머지 확인 + 핀 SHA 확보  [quick] [seq]

Wave 1 — HSI 계약/기반 (PR2: docs+contract-tests):
├── 1a [‖] 2. payload schema v2 명세+JSON schema [deep] · 3. terminal_reason taxonomy [quick]
│         · 4. 체크아웃 토폴로지/trust boundary 문서 [unspecified-high] · 5. iteration `>=max` 거부 + concurrency 통일 [quick]
└── 1b [seq, after 2] 6. 워크플로우 계약 테스트 하니스 확장 (shape + payload pos/neg) [unspecified-high]

Wave 2 — HSI CLI 이식 + config override (PR3+PR4):
├── 2a [seq] 7. setup/codex-review 패키지를 home-server-infra로 이식 [deep]
├── 2b [‖, after 7] 8. config.yml consumer-overridable [unspecified-high] · 9. 코멘트 loop 메모리 제거 [deep]
│         · 10. cross-run artifact 탐색 → state_run_id [deep]
└── 2c [seq, after 7–10] 11. 이식 경로 pytest green [unspecified-high]

Wave 3 — HSI 풀 스테이지 동작 (PR5: behavior, dry-run):
├── 3a [seq] 12. reusable entrypoint + per-stage 게이팅 스켈레톤 [deep]
├── 3b [‖, after 12] 13. review 5축 matrix [unspecified-high] · 14. design cluster matrix [unspecified-high]
│         · 16. issue 스테이지(terminal only) [DECISION][unspecified-low] · 18. state bundle by run_id [unspecified-high]
├── 3c [seq, after 14] 15. fix matrix(agents+merge+safety, dry-run, no push) [deep]
├── 3d [seq, after 13,14,15,16] 17. finalize-stage 정규화기 [deep]
└── 3e [seq, gate, after 13,14,15,18] 19. 단계별 parity 테스트 [unspecified-high]

Wave 4 — HSI live capability (PR6: live):
├── 4a [seq, after 15,17] 20. trusted 2-phase fix-push + stale-head 재검증 [deep]
├── 4b [‖, after 20] 21. updated_head_sha 검증+output [unspecified-high] · 24. fork/trust 강제 [unspecified-high]
│         · 25. 실패/종료 가시화 [unspecified-high]
├── 4c [seq, after 21] 22. repository_dispatch 연속 emit (App token) [deep]
├── 4d [seq, after 22] 23. dispatch ledger + caps + concurrency 통일 [deep]
└── 4e [seq, gate, after 22,23,24,25] 26. negative runtime + dry-run dispatch smoke 증거 [unspecified-high]

Wave 5 — Consumer 어댑터 dry-run (PR7: rs-builder, 전부 순차):
├── 5a [seq] 27. thin SHA-pinned pull_request_target 어댑터 (dry-run) [unspecified-high]
├── 5b [seq, after 27] 28. manual + repository_dispatch 어댑터 (default branch) [unspecified-high]
└── 5c [seq, after 27,28] 29. 구식 워크플로우 비활성 + dry-run/1회 live parity 증명 [deep]

Wave 6 — Cleanup (PR8: rs-builder):
└── 6a [‖, after 29] 30. 라벨 워크플로우+action 제거 [quick] · 31. setup/codex-review+라벨 ops 제거 [quick]
          · 32. repo 문서 갱신 [writing]

Wave FINAL — 4 병렬 리뷰 후 user okay:
└── [‖] F1. Plan compliance (oracle) · F2. Code/workflow quality (unspecified-high)
          · F3. Real manual QA — dry-run+live 루프 재현 (unspecified-high) · F4. Scope fidelity (deep)
→ 결과 종합 제시 → 사용자 명시적 okay

Critical Path: 1 → 2 → 7 → 12 → 14 → 15 → 17 → 20 → 21 → 22 → 23 → 26 → 27 → 28 → 29 → (30‖31‖32) → F1–F4 → okay
Max Concurrent: ~4 (Wave 1a: 2,3,4,5 / Wave 3b: 13,14,16,18)
Note: Wave 경계는 PR 머지(maintainer 게이트)로 직렬화됨 — AGENTS.md(PR 분리/직접 머지 금지) 준수상 의도적.
이 마이그레이션은 본질적으로 PR-게이트 직렬 흐름이며, 병렬성은 각 sub-wave 내부로 한정된다.
```

### Dependency Matrix (Wave / Task → Blocked By → Blocks)

- **1**: - → 2–6
- **2**: 1 → 6, 12, 17, 18, 22
- **3**: 1 → 17, 25
- **4**: 1 → 7, 12, 24
- **5**: 1 → 22, 23
- **6**: 1, 2 → (gate W2)
- **7**: 1, 4 → 8, 9, 10, 11, 12
- **8**: 7 → 13, 14, 15, 19, 27
- **9**: 7 → 17, 18
- **10**: 7 → 18, 22
- **11**: 7, 8, 9, 10 → (gate W3)
- **12**: 2, 4, 7 → 13, 14, 15, 16, 17
- **13**: 8, 12 → 19
- **14**: 8, 12 → 19
- **15**: 8, 12, 14 → 19, 20
- **16**: 12 → 25
- **17**: 2, 3, 9, 12, 13, 14, 15, 16 → 20, 21, 22
- **18**: 2, 9, 10 → 19, 22
- **19**: 13, 14, 15, 18 → (gate W4)
- **20**: 15, 17 → 21, 24
- **21**: 20 → 22
- **22**: 5, 10, 17, 18, 21 → 23, 27
- **23**: 5, 22 → 26
- **24**: 4, 20 → 26
- **25**: 3, 16 → 26
- **26**: 22, 23, 24, 25 → (gate W5)
- **27**: 8, 22, 26 → 28, 29
- **28**: 27 → 29
- **29**: 27, 28 → 30, 31, 32
- **30**: 29 → F1
- **31**: 29 → F1
- **32**: 29 → F1
- **F1–F4**: 30, 31, 32 → user okay

### Agent Dispatch Summary

- **Wave 0**: 1 task — 1 → `quick`
- **Wave 1**: 5 — 2 → `deep`, 3 → `quick`, 4 → `unspecified-high`, 5 → `quick`, 6 → `unspecified-high`
- **Wave 2**: 5 — 7 → `deep`, 8 → `unspecified-high`, 9 → `deep`, 10 → `deep`, 11 → `unspecified-high`
- **Wave 3**: 8 — 12 → `deep`, 13/14 → `unspecified-high`, 15 → `deep`, 16 → `unspecified-low`, 17 → `deep`, 18/19 → `unspecified-high`
- **Wave 4**: 7 — 20 → `deep`, 21 → `unspecified-high`, 22 → `deep`, 23 → `deep`, 24/25/26 → `unspecified-high`
- **Wave 5**: 3 — 27/28 → `unspecified-high`, 29 → `deep`
- **Wave 6**: 3 — 30/31 → `quick`, 32 → `writing`
- **FINAL**: 4 — F1 → `oracle`, F2 → `unspecified-high`, F3 → `unspecified-high`, F4 → `deep`

---

## TODOs

> Implementation + Test = ONE Task. EVERY task MUST have: Recommended Agent Profile + Parallelization + QA Scenarios.
> **FORMAT**: 태스크 라벨은 bare number(`1.`, `2.`...). Final wave는 `F1.`, `F2.`...
> **CROSS-REPO**: 태스크별 작업 위치 명시 — `[HSI]` = `/Users/dongwon/Documents/Programming/DongwonTTuna-Labs/home-server-infra/`,
> `[RS]` = `/Users/dongwon/Documents/Programming/DongwonTTuna-Labs/rs-builder-relayer-client/`.

- [x] 1. [HSI] PR #18을 main에 머지 확인하고 핀 대상 commit SHA 확보

  **What to do**:
  - maintainer가 home-server-infra PR #18(`work/codex-loop-reusable-workflow`)을 `main`에 머지하도록 준비/요청
    (CI green, actionlint, `pytest tests/workflows` 통과 확인). 에이전트는 머지 자체는 수행하지 않음.
  - 머지 후 `gh api repos/DongwonTTuna-Labs/home-server-infra/commits/main -q .sha`로 **머지 commit SHA**를 확보,
    `.omo/evidence/task-1-merge-sha.txt`에 기록(이후 모든 consumer가 SHA pin할 기준).
  - `main`에 `.github/workflows/codex-loop-{reusable,dispatch}.yml`이 존재하는지 확인(`repository_dispatch`는 default branch에서만 발화).

  **Must NOT do**:
  - 에이전트가 직접 PR 머지(AGENTS.md: "PR은 절대 직접 머지하지 말 것"). 머지는 maintainer 게이트.
  - reusable core 내용 수정(이번 PR에서는 머지/검증만). branch pin으로 후속 작업 진행 금지.

  **Recommended Agent Profile**:
  - **Category**: `quick` — 검증 + SHA 확보 위주의 경량 게이트 태스크.
  - **Skills**: 없음. **Omitted**: `customize-opencode`(opencode 설정 전용, 무관).

  **Parallelization**:
  - **Can Run In Parallel**: NO (전체 선행 게이트)
  - **Blocks**: 2–6 (모든 후속 Wave)
  - **Blocked By**: None (시작 태스크, 단 maintainer 머지 대기)

  **References**:
  - PR #18: `https://github.com/DongwonTTuna-Labs/home-server-infra/pull/18` (state=OPEN, base=main, head=work/codex-loop-reusable-workflow)
  - AGENTS.md(RS) "Operating Contract": PR 직접 머지 금지 / SHA pin 정책 — 동일 org 규율 적용.
  - 문서 근거: `home-server-infra/docs/codex-loop-reusable.md` Rollout 섹션 — "merge to default branch before dispatch".

  **Acceptance Criteria**:
  - [ ] `gh pr view 18 --repo DongwonTTuna-Labs/home-server-infra --json state -q .state` → `MERGED`
  - [ ] `.omo/evidence/task-1-merge-sha.txt`에 40자 SHA 기록됨
  - [ ] `gh api .../contents/.github/workflows/codex-loop-dispatch.yml?ref=main` → HTTP 200

  **QA Scenarios**:
  ```
  Scenario: PR #18 머지 + SHA 확보 (happy)
    Tool: Bash (gh)
    Preconditions: maintainer가 PR #18 머지 완료
    Steps:
      1. gh pr view 18 --repo DongwonTTuna-Labs/home-server-infra --json state,mergeCommit
      2. SHA=$(gh api repos/DongwonTTuna-Labs/home-server-infra/commits/main -q .sha); echo "$SHA" | tee .omo/evidence/task-1-merge-sha.txt
      3. gh api "repos/DongwonTTuna-Labs/home-server-infra/contents/.github/workflows/codex-loop-reusable.yml?ref=main" -q .sha
    Expected Result: state=MERGED, 40-char SHA 기록, reusable yml이 main에 존재(HTTP 200)
    Failure Indicators: state!=MERGED, 빈 SHA, 404
    Evidence: .omo/evidence/task-1-merge-sha.txt

  Scenario: 미머지 상태 방어 (negative)
    Tool: Bash (gh)
    Preconditions: PR #18 아직 OPEN
    Steps:
      1. gh pr view 18 --repo DongwonTTuna-Labs/home-server-infra --json state -q .state
      2. 결과가 MERGED가 아니면 후속 Wave 진행 차단(STOP)
    Expected Result: OPEN이면 명시적으로 중단, 후속 태스크 미진행
    Evidence: .omo/evidence/task-1-merge-gate-error.txt
  ```

  **Commit**: NO (머지는 maintainer; 에이전트 산출물은 evidence 파일뿐)

- [x] 2. [HSI] 루프 상태머신 + payload schema v2 명세(`schema_version`/`state_run_id`/`state_artifact_name`) + JSON schema

  **What to do**:
  - `docs/codex-loop-reusable.md`의 Payload Schema를 **v2**로 갱신: 기존 키 + `schema_version`(int), `state_run_id`(string),
    `state_artifact_name`(string), 그리고 stages/transitions/terminal/caps를 표로 정의(review→design→fix→review… until LGTM).
  - 루프 상태머신을 명세: 허용 전이, 종료상태(LGTM/terminal_reason), iteration/dispatch caps, stale-head 재검증 지점.
  - 머신리더블 schema 추가: `schemas/codex-loop-dispatch-payload.v2.schema.json`(또는 setup 이식 경로 규칙에 맞춰).

  **Must NOT do**:
  - free-form state를 payload에 추가 금지(버전드 키만). 라벨/코멘트 전이 재도입 금지. 모델 출력/secret을 payload에 넣지 않음.

  **Recommended Agent Profile**:
  - **Category**: `deep` — 상태머신 계약 설계가 이후 모든 단계의 토대.
  - **Skills**: 없음. **Omitted**: `openspec-*`(이 작업은 spec 산출물 강제 아님), `customize-opencode`(무관).

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 1a (with 3, 4, 5)
  - **Blocks**: 6, 12, 17, 18, 22
  - **Blocked By**: 1

  **References**:
  - `home-server-infra/.github/workflows/codex-loop-dispatch.yml:31-114`(payload `validate_payload` step: require_non_empty + stage/iteration 검증 패턴).
  - `home-server-infra/.github/workflows/codex-loop-reusable.yml:50-64`(현 outputs 계약 next_stage/lgtm/should_redispatch/terminal_reason — v2가 확장할 대상).
  - `setup/codex-review/schemas/loop-state.v1.schema.json`, `setup/codex-review/schemas/reentry-loop-state.v1.schema.json`(기존 loop state 형태).
  - `setup/codex-review/src/codex_review/loop/state.py:124-130`(loop-state.v1 schema_version/head_sha 필드 — 신규 state pointer 정합 기준).

  **Acceptance Criteria**:
  - [ ] `docs/codex-loop-reusable.md`에 v2 payload 표 + state machine 표 존재(`schema_version`, `state_run_id`, `state_artifact_name` 포함)
  - [ ] `python3 -c "import json;json.load(open('schemas/codex-loop-dispatch-payload.v2.schema.json'))"` → 에러 없음
  - [ ] terminal/transition/caps가 문서에 명시(LGTM terminal 포함)

  **QA Scenarios**:
  ```
  Scenario: v2 schema 유효성 + 키 존재 (happy)
    Tool: Bash (python3/jq)
    Preconditions: 문서/스키마 작성됨
    Steps:
      1. python3 -c "import json;d=json.load(open('schemas/codex-loop-dispatch-payload.v2.schema.json'));print(sorted(d['properties']))"
      2. grep -E "schema_version|state_run_id|state_artifact_name" docs/codex-loop-reusable.md
    Expected Result: 3개 신규 키 모두 schema properties + 문서에 존재
    Evidence: .omo/evidence/task-2-schema-keys.txt

  Scenario: free-form state 금지 회귀 방어 (negative)
    Tool: Bash (jq)
    Steps:
      1. jq '.additionalProperties' schemas/codex-loop-dispatch-payload.v2.schema.json
    Expected Result: additionalProperties=false (자유 키 차단)
    Evidence: .omo/evidence/task-2-additionalprops-error.txt
  ```

  **Commit**: YES (groups with PR2)
  - Message: `docs(codex-loop): payload v2 contract + loop state machine`
  - Files: `docs/codex-loop-reusable.md`, `schemas/codex-loop-dispatch-payload.v2.schema.json`
  - Pre-commit: `python3 -m json.tool schemas/codex-loop-dispatch-payload.v2.schema.json`

- [x] 3. [HSI] `terminal_reason` taxonomy enum + 문서화

  **What to do**:
  - Oracle 권고 taxonomy를 단일 출처로 정의(enum): `lgtm, dry_run, no_fix_needed, no_fix_changes, empty_patch,
    validation_failed, tests_failed, semantic_safety_missing, semantic_safety_rejected, semantic_safety_hash_mismatch,
    policy_rejected, stale_head, base_ref_mismatch, pr_closed, fork_pr, untrusted_repository_owner, untrusted_requester,
    missing_app_credentials, app_token_scope_invalid, push_failed, pushed_unverified, dispatch_failed, dispatch_duplicate,
    max_iterations, oscillation_detected, artifact_missing, artifact_schema_invalid, model_output_invalid, stage_failed`
    (issue 단계 유지 시 `issue_created` 포함).
  - `docs/codex-loop-reusable.md` "Loop Termination"에 표로 문서화 + 머신리더블(예: `schemas/terminal-reason.v1.json` 또는 CLI enum).

  **Must NOT do**:
  - terminal_reason를 라벨/코멘트로 표기 금지(상태가 아니라 출력/artifact/summary로만). 임의 free-text reason 남발 금지.

  **Recommended Agent Profile**:
  - **Category**: `quick` — enum + 문서표 정의(저위험).
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 1
  - **Blocks**: 17, 25
  - **Blocked By**: 1

  **References**:
  - `setup/codex-review/src/codex_review/stages/push/push.py:49`(remote_head_sha/expected_head_sha/verified — `pushed_unverified` 근거).
  - `.github/workflows/codex-fix.yml:465-481`(no_fix_needed/loop terminal/needs-issue 라우팅 → terminal_reason 매핑 원천).
  - `.github/workflows/codex-review.yml:545-565`(stop_lgtm/stop_needs_human/run_design 라우팅 → terminal 매핑).
  - `setup/codex-review/src/codex_review/loop/state.py:124`(loop-state.v1 schema_version — taxonomy 일관성 기준).

  **Acceptance Criteria**:
  - [ ] 모든 reason이 단일 enum 출처에 정의(중복/오타 없음)
  - [ ] 문서표에 각 reason의 의미 1줄 + 발생 단계 명시
  - [ ] `grep -c` 로 핵심 reason(`stale_head`,`max_iterations`,`fork_pr`,`no_fix_changes`) 문서 존재 확인

  **QA Scenarios**:
  ```
  Scenario: taxonomy 단일 출처 + 문서 동기화 (happy)
    Tool: Bash (grep/python3)
    Steps:
      1. python3 -c "import json;print(len(json.load(open('schemas/terminal-reason.v1.json'))['enum']))"
      2. for r in lgtm stale_head max_iterations fork_pr no_fix_changes; do grep -q "$r" docs/codex-loop-reusable.md || echo "MISSING $r"; done
    Expected Result: enum 길이 ≥ 25, MISSING 출력 없음
    Evidence: .omo/evidence/task-3-taxonomy.txt

  Scenario: 라벨 표기 회귀 방어 (negative)
    Tool: Bash (rg)
    Steps:
      1. rg -n "add-label|labels\[\]|gh .* label" docs/codex-loop-reusable.md
    Expected Result: terminal 처리에 라벨 사용 매치 0
    Evidence: .omo/evidence/task-3-no-label-error.txt
  ```

  **Commit**: YES (PR2)
  - Message: `docs(codex-loop): terminal_reason taxonomy`
  - Files: `docs/codex-loop-reusable.md`, `schemas/terminal-reason.v1.json`
  - Pre-commit: `python3 -m json.tool schemas/terminal-reason.v1.json`

- [x] 4. [HSI] 체크아웃 토폴로지 / trust boundary 계약 문서 (4 worktrees)

  **What to do**:
  - `docs/codex-loop-reusable.md`에 Oracle 권고 4-worktree 토폴로지를 명세:
    `trusted-core/`(home-server-infra@pinned SHA → CLI 설치), `target-base/`(consumer base ref, trusted context),
    `pr-head/`(consumer PR head, **데이터 전용**, secrets 하 스크립트 실행 금지), `pr-head-write/`(fix push 전용, 가드 통과 후).
  - 각 worktree의 checkout 파라미터(`repository`, `ref`, `persist-credentials: false`), 신뢰 등급, 허용 동작을 표로.
  - SHA 파생 규칙 명세: reusable core가 자기 CLI를 호출 워크플로우 ref와 동일 SHA에서 가져오는 방법
    (`github.workflow_ref` 파싱; 모호하면 consumer adapter가 `core_sha` 입력으로 전달 + ref 일치 assert).

  **Must NOT do**:
  - PR head 트리에서 스크립트/액션/패키지 매니저 실행 또는 거기서 CLI 설치 금지. branch ref로 CLI 소싱 금지(SHA만).

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — 보안 경계 계약, 정확성이 핵심.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 1
  - **Blocks**: 7, 12, 24
  - **Blocked By**: 1

  **References**:
  - `rs-builder.../.github/actions/setup-codex-review/action.yml:7-13`(helper를 `github.workflow_sha`로 checkout하는 기존 패턴).
  - `rs-builder.../.github/workflows/codex-review.yml:290-296`(PR head를 별도 path + `persist-credentials:false`로 checkout).
  - `rs-builder.../.github/workflows/codex-fix.yml:255-260, 366-376`(trusted base + pr-head 분리, fix push용 checkout).
  - Oracle Part B-1 체크아웃 토폴로지 권고(상단 Oracle Review).

  **Acceptance Criteria**:
  - [ ] 문서에 4개 worktree 각각의 repository/ref/persist-credentials/신뢰등급/허용동작 표 존재
  - [ ] CLI SHA 소싱 규칙(`github.workflow_ref` 또는 `core_sha` assert) 명세
  - [ ] "PR head = data only, no script exec under secrets" 명시 문장 존재

  **QA Scenarios**:
  ```
  Scenario: 토폴로지 계약 완전성 (happy)
    Tool: Bash (grep)
    Steps:
      1. for w in trusted-core target-base pr-head pr-head-write; do grep -q "$w" docs/codex-loop-reusable.md || echo "MISSING $w"; done
      2. grep -E "persist-credentials: false" docs/codex-loop-reusable.md
      3. grep -iE "data only|never .* execute|스크립트 실행 금지" docs/codex-loop-reusable.md
    Expected Result: MISSING 없음, persist-credentials/데이터전용 문구 존재
    Evidence: .omo/evidence/task-4-topology.txt

  Scenario: branch-ref CLI 소싱 금지 명시 (negative)
    Tool: Bash (grep)
    Steps:
      1. grep -iE "pin .* sha|sha only|branch pin .* forbidden|브랜치 pin" docs/codex-loop-reusable.md
    Expected Result: SHA-only 소싱 규칙 문구 존재
    Evidence: .omo/evidence/task-4-sha-only.txt
  ```

  **Commit**: YES (PR2)
  - Message: `docs(codex-loop): checkout topology + trust boundary`
  - Files: `docs/codex-loop-reusable.md`
  - Pre-commit: `actionlint .github/workflows/codex-loop-reusable.yml`(영향 없음 확인)

- [x] 5. [HSI] reusable core iteration `>= max` 거부 + concurrency group 통일

  **What to do**:
  - `codex-loop-reusable.yml` validate 단계의 iteration 체크를 수정: 다음 dispatch 기준 `iteration >= max_iterations`를
    `max_iterations` terminal로 거부(현재 `> max`만 거부 = off-by-one). 단일 실행 내 마지막 단계 실행 정책을 문서/주석에 명시.
  - core(`codex-loop-${{ inputs.correlation_id }}`)와 dispatch adapter(`codex-loop-${pr}-${sha}`)의 concurrency group을
    통일(권장: `correlation_id` 기준, ordered면 `cancel-in-progress:false`; 동일 PR+head 중복 kickoff만 cancel).

  **Must NOT do**:
  - iteration 의미를 느슨하게 두어 runaway 허용 금지. concurrency를 dry-run/live가 공유해 충돌하게 두지 않음.

  **Recommended Agent Profile**:
  - **Category**: `quick` — 국소 수정(조건식 + concurrency 키), 단 의미는 명확히.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 1
  - **Blocks**: 22, 23
  - **Blocked By**: 1

  **References**:
  - `home-server-infra/.github/workflows/codex-loop-reusable.yml:138-139`(`(( INPUT_ITERATION > INPUT_MAX_ITERATIONS ))` → `max-iterations-exceeded`; off-by-one 수정 대상).
  - `home-server-infra/.github/workflows/codex-loop-reusable.yml:67-68`(core concurrency `group: codex-loop-${{ inputs.correlation_id }}`).
  - `home-server-infra/.github/workflows/codex-loop-dispatch.yml:9-10`(dispatch concurrency `group: codex-loop-${pr}-${head_sha}` — 통일 대상).

  **Acceptance Criteria**:
  - [ ] iteration 체크가 `>= max_iterations`에서 `max_iterations` terminal 산출
  - [ ] core/dispatch concurrency group 표현식 일치(또는 문서화된 의도적 차등)
  - [ ] `actionlint codex-loop-reusable.yml codex-loop-dispatch.yml` 0 error

  **QA Scenarios**:
  ```
  Scenario: off-by-one 차단 (happy)
    Tool: Bash (grep/actionlint)
    Steps:
      1. grep -nE ">=|ge | -ge " .github/workflows/codex-loop-reusable.yml
      2. actionlint .github/workflows/codex-loop-reusable.yml .github/workflows/codex-loop-dispatch.yml
    Expected Result: iteration>=max 거부 로직 존재, actionlint 0 error
    Evidence: .omo/evidence/task-5-iteration.txt

  Scenario: concurrency 통일 확인 (happy/negative)
    Tool: Bash (grep)
    Steps:
      1. grep -nA1 "concurrency:" .github/workflows/codex-loop-reusable.yml .github/workflows/codex-loop-dispatch.yml
    Expected Result: group 표현식이 correlation_id 기준으로 정합(불일치 시 FAIL)
    Evidence: .omo/evidence/task-5-concurrency.txt
  ```

  **Commit**: YES (PR2)
  - Message: `fix(codex-loop): reject iteration>=max, unify concurrency group`
  - Files: `.github/workflows/codex-loop-reusable.yml`, `.github/workflows/codex-loop-dispatch.yml`
  - Pre-commit: `actionlint .github/workflows/codex-loop-*.yml`

- [x] 6. [HSI] 워크플로우 계약 테스트 하니스 확장 (shape + payload pos/neg)

  **What to do**:
  - PR #18의 `tests/workflows/test_workflow_contracts.py`를 확장: reusable에 `workflow_call`/필수 inputs(타입 포함)/secrets/
    least-privilege permissions 존재, dispatch type이 정확히 `codex-loop`, **라벨 트리거/PAT secret 부재**, docs에 branch-pin 예시 부재 검증.
  - payload v2 검증 pos/neg 테스트: valid review payload 수락, invalid stage/누락 head_sha/iteration>=max/stale SHA/closed PR/
    fork fix/중복·역행 iteration 거부.

  **Must NOT do**:
  - 실제 네트워크/라이브 dispatch 호출 금지(이 태스크는 정적 계약 + payload 검증 단위). 테스트를 통과시키려 검증 로직 약화 금지.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — 계약 테스트 정확성이 회귀 방어의 핵심.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: NO — Wave 1b (task 2 schema 산출 이후 순차)
  - **Blocks**: Wave 2 게이트
  - **Blocked By**: 1, 2

  **References**:
  - `home-server-infra/tests/workflows/test_workflow_contracts.py`(PR#18, 19 tests) + `tests/workflows/fixtures/unsafe-*.yml`.
  - `home-server-infra/.github/actionlint.yaml`(self-hosted runner labels).
  - task 2 산출 `schemas/codex-loop-dispatch-payload.v2.schema.json`.

  **Acceptance Criteria**:
  - [ ] `uvx pytest tests/workflows -q` 전부 통과(신규 pos/neg 포함)
  - [ ] 라벨 트리거/PAT secret/branch-pin 예시 부재를 assert하는 테스트 존재
  - [ ] payload neg 케이스(invalid stage, missing head_sha, iteration>=max, fork fix, stale) 각각 테스트 존재

  **QA Scenarios**:
  ```
  Scenario: 계약 테스트 green (happy)
    Tool: Bash (pytest)
    Steps:
      1. uvx pytest tests/workflows -q
    Expected Result: all passed (PR#18 19개 + 신규 추가분)
    Evidence: .omo/evidence/task-6-pytest.txt

  Scenario: 잘못된 payload 거부 (negative)
    Tool: Bash (pytest -k)
    Steps:
      1. uvx pytest tests/workflows -q -k "invalid or fork or stale or iteration"
    Expected Result: 모든 neg 테스트가 "거부"를 검증하며 통과
    Evidence: .omo/evidence/task-6-negative.txt
  ```

  **Commit**: YES (PR2)
  - Message: `test(codex-loop): expand workflow + payload contract tests`
  - Files: `tests/workflows/test_workflow_contracts.py`, `tests/workflows/fixtures/*`
  - Pre-commit: `uvx pytest tests/workflows -q`

- [x] 7. [HSI] `setup/codex-review` 패키지를 home-server-infra로 이식 (git mv + pyproject/경로)

  **What to do**:
  - `rs-builder.../setup/codex-review/` 전체(src/codex_review, bin/codex-review, schemas, prompts, config.yml, pyproject.toml,
    tests)를 home-server-infra의 합의 경로(예: `setup/codex-review/`)로 이식. `bin/codex-review`의 PYTHONPATH/엔트리포인트 유지.
  - home-server-infra용 setup composite action(또는 reusable core 내 install 단계) 추가: `trusted-core/`(home-server-infra@SHA)
    에서 `pip install -e setup/codex-review` 후 `codex-review` CLI 사용 가능하게.
  - 이식 후 `import` 경로/패키지 메타데이터(egg-info 제외)와 schema 상대경로가 깨지지 않는지 정리.

  **Must NOT do**:
  - 이식 중 로직/프롬프트/스키마 "개선" 금지(순수 relocation + 경로 보정). consumer PR head에서 설치 금지.
  - rs-builder 쪽 원본은 이 태스크에서 삭제하지 않음(제거는 Wave 6, parity 증명 후).

  **Recommended Agent Profile**:
  - **Category**: `deep` — 12k LOC 패키지 relocation + 런타임 소싱 결선, 파손 지점 많음.
  - **Skills**: 없음. **Omitted**: `customize-opencode`(무관).

  **Parallelization**:
  - **Can Run In Parallel**: NO (Wave 2 선행 — 8/9/10/11이 이식 결과에 의존)
  - **Blocks**: 8, 9, 10, 11, 12
  - **Blocked By**: 1, 4

  **References**:
  - `rs-builder.../setup/codex-review/`(bin/codex-review, src/codex_review/*, schemas/*, prompts/*, config.yml, pyproject.toml, tests/).
  - `rs-builder.../.github/actions/setup-codex-review/action.yml:14-21`(python setup + `pip install -e` 패턴).
  - `bin/codex-review`(PYTHONPATH + `python3 -m codex_review.cli`).
  - Oracle Part B-7 PR3(setup) decomposition.

  **Acceptance Criteria**:
  - [ ] home-server-infra에서 `pip install -e setup/codex-review` 성공
  - [ ] `codex-review --help`(또는 알려진 서브커맨드) 정상 실행
  - [ ] schema/prompt 상대경로 로딩 정상(샘플 서브커맨드 1개 실행으로 확인)

  **QA Scenarios**:
  ```
  Scenario: 이식 패키지 설치/실행 (happy)
    Tool: Bash (pip/python)
    Preconditions: home-server-infra@main에 패키지 이식됨
    Steps:
      1. cd <hsi> && python3 -m pip install --disable-pip-version-check -e setup/codex-review
      2. setup/codex-review/bin/codex-review --help 2>&1 | head
      3. python3 -c "import codex_review; print(codex_review.__file__)"
    Expected Result: 설치 성공, CLI 도움말 출력, import 경로가 hsi 트리
    Evidence: .omo/evidence/task-7-install.txt

  Scenario: schema 상대경로 무결성 (negative-guard)
    Tool: Bash
    Steps:
      1. setup/codex-review/bin/codex-review schema openai-strict --schema review-axis-findings.v1 --out /tmp/s.json
      2. python3 -m json.tool /tmp/s.json >/dev/null && echo OK
    Expected Result: 스키마 생성 성공(경로 깨짐 없음)
    Evidence: .omo/evidence/task-7-schema-path-error.txt
  ```

  **Commit**: YES (PR3)
  - Message: `chore(codex-loop): port codex-review CLI into home-server-infra`
  - Files: `setup/codex-review/**`, setup composite action
  - Pre-commit: `python3 -m pip install -e setup/codex-review && uvx pytest setup/codex-review/tests -q`

- [x] 8. [HSI] `config.yml` repo-고정 경로 → consumer-overridable 입력/설정

  **What to do**:
  - `config.yml`의 repo-특화 값(경로/금지파일/docs 컨텍스트 등)을 consumer가 override 가능하게 추출:
    reusable workflow 입력 또는 consumer가 자기 repo에 두는 `.codex-loop.yml` 설정으로 분리. 기본값은 안전한 generic.
  - CLI가 override 설정을 로드하는 경로 결선(env/입력 → config merge). 미지정 시 동작이 깨지지 않게 디폴트.

  **Must NOT do**:
  - rs-builder 전용 경로/금지파일을 home-server-infra core에 하드코딩 유지 금지. secrets를 config로 옮기지 않음.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — org-reusable화의 핵심(설정 외부화).
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 2 (task 7 이후)
  - **Blocks**: 13, 14, 15, 19, 27
  - **Blocked By**: 7

  **References**:
  - `rs-builder.../setup/codex-review/config.yml:~71`(repo-고정 경로/forbidden files — Oracle 지목).
  - `rs-builder.../setup/codex-review/src/codex_review/`(config 로더 위치 — context 빌더가 config 소비).
  - Oracle Part B-7 PR4(config overridable) decomposition.

  **Acceptance Criteria**:
  - [ ] config의 repo-특화 키가 override 가능(입력 또는 consumer `.codex-loop.yml`)
  - [ ] override 미지정 시 generic 기본값으로 정상 동작
  - [ ] override 적용 단위 테스트 추가 + 통과

  **QA Scenarios**:
  ```
  Scenario: consumer override 적용 (happy)
    Tool: Bash (pytest/python)
    Steps:
      1. uvx pytest setup/codex-review/tests -q -k "config or override"
      2. python3 -c "from codex_review.config import load_config; print(load_config(override={'paths':{'x':'y'}}))"
    Expected Result: override 병합 동작, 테스트 통과
    Evidence: .omo/evidence/task-8-override.txt

  Scenario: 기본값 fallback (negative-guard)
    Tool: Bash
    Steps:
      1. (override 없이) 컨텍스트 빌드 서브커맨드 1개 dry 실행
    Expected Result: 누락 override에도 generic 기본값으로 무중단
    Evidence: .omo/evidence/task-8-default-fallback.txt
  ```

  **Commit**: YES (PR4)
  - Message: `feat(codex-loop): consumer-overridable config`
  - Files: `setup/codex-review/config.yml`, `setup/codex-review/src/codex_review/config*.py`, tests
  - Pre-commit: `uvx pytest setup/codex-review/tests -q -k config`

- [x] 9. [HSI] 코멘트 기반 loop 메모리(`loop/state.py`) 제거 → artifact/payload 상태

  **What to do**:
  - `loop/state.py`의 sticky-comment(`codex-review:loop-state` 마커) 읽기/쓰기 경로를 제거하거나 legacy-only로 격리.
  - 루프 상태를 **payload(state pointer) + state-bundle artifact**에서 읽도록 재결선(코멘트/라벨 의존 0).
  - 코멘트 메모리를 호출하던 모든 지점을 새 state 소스로 교체. 호출 그래프 검증.

  **Must NOT do**:
  - 코멘트/라벨을 다른 이름으로 상태 저장에 재도입 금지. 기존 리뷰 로직 의미 변경 금지(상태 전달 방식만 교체).

  **Recommended Agent Profile**:
  - **Category**: `deep` — 상태 소스 교체는 광범위 영향(호출처 다수), 정확성 핵심.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 2 (task 7 이후)
  - **Blocks**: 17, 18
  - **Blocked By**: 7

  **References**:
  - `setup/codex-review/src/codex_review/loop/state.py:18-21`(read_loop_state_from_comments), `:38-44`(render/write_loop_state_comment), `:50`(sticky comment 기반 push history) — 제거/legacy화 대상.
  - `setup/codex-review/src/codex_review/github/comments.py`(upsert_sticky_comment), `.../github/markers.py`(parse/render_marker) — 코멘트 의존 호출처(state.py:12-13 import).
  - `.github/workflows/codex-review.yml:102`(`loop read-state`), `.github/workflows/codex-fix.yml:412-417`(record-push into loop state history).
  - `setup/codex-review/schemas/loop-state.v1.schema.json`, `setup/codex-review/schemas/reentry-loop-state.v1.schema.json`.

  **Acceptance Criteria**:
  - [ ] `rg "loop-state|sticky|comment" src/codex_review/loop/` → 활성 코드에서 코멘트 상태 경로 0(또는 legacy 가드 뒤)
  - [ ] loop state read/write가 artifact/payload 기반으로 동작(단위 테스트 통과)
  - [ ] 코멘트 메모리 호출처 전부 새 소스로 교체됨(grep 검증)

  **QA Scenarios**:
  ```
  Scenario: artifact/payload 기반 상태 동작 (happy)
    Tool: Bash (pytest)
    Steps:
      1. uvx pytest setup/codex-review/tests -q -k "loop or state"
    Expected Result: 새 state 경로 테스트 통과
    Evidence: .omo/evidence/task-9-state.txt

  Scenario: 코멘트 상태 잔존 회귀 방어 (negative)
    Tool: Bash (rg)
    Steps:
      1. rg -n "codex-review:loop-state|issue_comment|pr comment .* state" setup/codex-review/src/codex_review/loop/
    Expected Result: 활성 경로 매치 0 (legacy 가드/주석만 허용)
    Evidence: .omo/evidence/task-9-no-comment-state-error.txt
  ```

  **Commit**: YES (PR3)
  - Message: `refactor(codex-loop): remove comment-backed loop memory`
  - Files: `setup/codex-review/src/codex_review/loop/*.py`, tests
  - Pre-commit: `uvx pytest setup/codex-review/tests -q -k loop`

- [x] 10. [HSI] cross-run artifact 탐색(`gh run list --headSha`) → 명시적 `state_run_id`

  **What to do**:
  - 단계 간 artifact 취득을 `gh run list ... --headSha` 발견식에서 **payload의 `state_run_id` + `download-artifact run-id`**
    명시 취득으로 교체. design→fix가 review/design 산출 run을 정확히 지목하도록.
  - state-bundle 명명 규칙(`state_artifact_name`) 정의 + CLI/워크플로우가 일관되게 사용.

  **Must NOT do**:
  - head_sha로 성공 run을 추정 검색 금지(재시도/중복/동일-head 재dispatch에서 오선택 위험). 대용량/모델출력을 outputs로 넘기지 않음(artifact 사용).

  **Recommended Agent Profile**:
  - **Category**: `deep` — 상태 전송 모델의 정합성 보장.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 2 (task 7 이후)
  - **Blocks**: 18, 22
  - **Blocked By**: 7

  **References**:
  - `rs-builder.../.github/workflows/codex-design.yml:77-83`(`gh run list --workflow=codex-review.yml ... headSha` 발견식).
  - `rs-builder.../.github/workflows/codex-fix.yml:83-89`(design run 발견식), `codex-issue.yml:61-69`(find_run).
  - task 2 payload v2(`state_run_id`/`state_artifact_name`), Oracle Part B-2 상태 전송 권고.

  **Acceptance Criteria**:
  - [ ] cross-run 취득 코드가 `state_run_id` 입력을 사용(headSha 검색 0)
  - [ ] `rg "gh run list .*headSha|--headSha"` → 새 코드 경로에서 매치 0
  - [ ] state-bundle 업/다운로드 명명이 `state_artifact_name`로 통일(단위 테스트)

  **QA Scenarios**:
  ```
  Scenario: 명시적 run_id 취득 (happy)
    Tool: Bash (pytest/grep)
    Steps:
      1. uvx pytest setup/codex-review/tests -q -k "state_run_id or artifact"
      2. rg -n "state_run_id|run-id" setup/codex-review/src .github/workflows 2>/dev/null | head
    Expected Result: state_run_id 사용 경로 존재, 테스트 통과
    Evidence: .omo/evidence/task-10-runid.txt

  Scenario: headSha 발견식 회귀 방어 (negative)
    Tool: Bash (rg)
    Steps:
      1. rg -n "run list.*headSha|--headSha" setup/codex-review/src .github/workflows
    Expected Result: 신규 경로 매치 0
    Evidence: .omo/evidence/task-10-no-headsha-error.txt
  ```

  **Commit**: YES (PR3)
  - Message: `refactor(codex-loop): explicit state_run_id artifact transport`
  - Files: `setup/codex-review/src/codex_review/**`, tests
  - Pre-commit: `uvx pytest setup/codex-review/tests -q -k state_run_id`

- [x] 11. [HSI] 이식 경로에서 기존 pytest green 보증

  **What to do**:
  - 이식/리팩터(7–10) 후 `setup/codex-review/tests`(unit + fixtures + workflow) 전체를 home-server-infra 경로에서 실행해 green.
  - 경로 변경/상태 소스 교체로 깨진 테스트를 **테스트 삭제 없이** 정합되게 수정(픽스처 경로 등). 새 동작에 맞는 회귀 픽스처 보강.

  **Must NOT do**:
  - 통과시키려 테스트 삭제/광범위 mock/스킵 금지(AGENTS.md). flaky면 원인 기록 후 처리.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — 회귀 안전망 복구.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: NO (Wave 2 마감 게이트 — 7/8/9/10 통합 검증)
  - **Blocks**: Wave 3 게이트
  - **Blocked By**: 7, 8, 9, 10

  **References**:
  - `rs-builder.../setup/codex-review/tests/{unit,fixtures,workflow}`, `pyproject.toml`(pytest 설정).
  - `docs/TESTING.md`(RS) 디시플린(테스트 삭제 금지 등).

  **Acceptance Criteria**:
  - [ ] `uvx pytest setup/codex-review/tests -q` 전부 통과(스킵 사유 명시 없는 skip 0)
  - [ ] 삭제된 테스트 0(diff로 확인), 수정은 경로/상태 정합 목적만

  **QA Scenarios**:
  ```
  Scenario: 전체 pytest green (happy)
    Tool: Bash (pytest)
    Steps:
      1. uvx pytest setup/codex-review/tests -q | tee .omo/evidence/task-11-pytest.txt
    Expected Result: all passed, unexpected skip 0
    Evidence: .omo/evidence/task-11-pytest.txt

  Scenario: 테스트 삭제 회귀 방어 (negative)
    Tool: Bash (git)
    Steps:
      1. git -C <hsi> diff --stat origin/main -- setup/codex-review/tests
    Expected Result: 테스트 파일 삭제(-) 없음
    Evidence: .omo/evidence/task-11-no-test-deletion.txt
  ```

  **Commit**: YES (PR3)
  - Message: `test(codex-loop): green pytest after relocation`
  - Files: `setup/codex-review/tests/**`
  - Pre-commit: `uvx pytest setup/codex-review/tests -q`

- [x] 12. [HSI] reusable 워크플로우 entrypoint + per-stage job 게이팅 스켈레톤

  **What to do**:
  - `codex-loop-reusable.yml`을 단일 entrypoint로 두고 `stage` 입력에 따라 stage-specific job 그룹을
    `if: inputs.stage == 'review'` 등으로 게이팅하는 골격 구성. 공통 선두 job: validate → trust-and-stale-guard →
    checkout 토폴로지(trusted-core/target-base/pr-head) → setup-relay.
  - 공통 `finalize-stage` job 자리(정규화 출력) 예약. 각 stage 그룹은 placeholder job으로 시작(실 로직은 13–16).

  **Must NOT do**:
  - 단일 generic 프롬프트로 rich 파이프라인 대체 금지(PR#18 placeholder 패턴 유지 금지). PR head를 secrets 하 실행 금지.

  **Recommended Agent Profile**:
  - **Category**: `deep` — 워크플로우 골격이 13–19 전부의 토대.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: NO (Wave 3 선행)
  - **Blocks**: 13, 14, 15, 16, 17
  - **Blocked By**: 2, 4, 7

  **References**:
  - `home-server-infra/.github/workflows/codex-loop-reusable.yml`(validate/trust/setup-relay/run-stage/finalize 골격).
  - `rs-builder.../.github/workflows/codex-review.yml`(stage별 job 구성 패턴), task 4(체크아웃 토폴로지), task 7(CLI 소싱).
  - Oracle Part B-6(stage 내부 fan-out + 공통 finalize-stage 권고).

  **Acceptance Criteria**:
  - [ ] reusable에 review/design/fix/issue stage 그룹이 `if: inputs.stage==...`로 게이팅됨
  - [ ] 공통 checkout 토폴로지(trusted-core/target-base/pr-head) + setup-relay job 존재
  - [ ] `actionlint codex-loop-reusable.yml` 0 error, 골격에서 stage별 placeholder job 동작

  **QA Scenarios**:
  ```
  Scenario: stage 게이팅 골격 (happy)
    Tool: Bash (actionlint/yq)
    Steps:
      1. actionlint .github/workflows/codex-loop-reusable.yml
      2. python3 -c "import yaml;d=yaml.safe_load(open('.github/workflows/codex-loop-reusable.yml'));print([j for j in d['jobs']])"
    Expected Result: actionlint 0 error, stage-gated job 존재
    Evidence: .omo/evidence/task-12-skeleton.txt

  Scenario: generic placeholder 잔존 방어 (negative)
    Tool: Bash (rg)
    Steps:
      1. rg -n "Run Codex loop stage .* for PR|dry-run-placeholder" .github/workflows/codex-loop-reusable.yml
    Expected Result: PR#18 generic 프롬프트 placeholder 제거(매치 0 또는 dry-run 분기 한정)
    Evidence: .omo/evidence/task-12-no-generic-error.txt
  ```

  **Commit**: YES (PR5)
  - Message: `feat(codex-loop): stage-gated reusable workflow skeleton`
  - Files: `.github/workflows/codex-loop-reusable.yml`
  - Pre-commit: `actionlint .github/workflows/codex-loop-reusable.yml`

- [x] 13. [HSI] review 스테이지 내부 matrix(5축) 결선

  **What to do**:
  - `stage=review` job 그룹에 기존 멀티축 리뷰 파이프라인을 결선: collect_threads → triage(resolve_gate) →
    review_axes(matrix: correctness/security/performance/test-coverage/domain) → combine → techlead → publish/route.
  - 각 step은 trusted-core CLI(`codex-review review …`)를 pr-head(데이터) 대상으로 실행. relay 토큰은 setup-relay 산출 사용.
  - 결과를 state-bundle artifact로 업로드(`state_artifact_name`) + finalize-stage가 소비할 형태로 정규화. dry-run에서 posting 비활성.

  **Must NOT do**:
  - 5축을 단일 프롬프트로 축약 금지. PR head 스크립트 실행 금지. 라벨/코멘트로 결과 라우팅 금지(route는 outputs/artifact).

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — 기존 로직 결선(신규 로직 아님), 정확한 와이어링 핵심.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 3 (with 14)
  - **Blocks**: 19
  - **Blocked By**: 8, 12

  **References**:
  - `rs-builder.../.github/workflows/codex-review.yml:118-498`(collect_threads/triage_threads/apply_threads/review_axes(matrix:280-335)/combine/techlead/publish).
  - `setup/codex-review/src/codex_review/stages/{resolve_gate,review,techlead}` + `schemas/review-axis-findings.v1.schema.json`.
  - `setup/codex-review/prompts/{review,resolve_gate,techlead}`.

  **Acceptance Criteria**:
  - [ ] review job이 5축 matrix로 fan-out + combine + techlead 단계 포함
  - [ ] dry-run에서 결정론적 산출(state-bundle artifact 업로드), posting 비활성
  - [ ] `actionlint` 0 error + review 단위/parity 픽스처(샘플) 통과

  **QA Scenarios**:
  ```
  Scenario: review 5축 결선 (happy)
    Tool: Bash (actionlint/yq/pytest)
    Steps:
      1. actionlint .github/workflows/codex-loop-reusable.yml
      2. python3 -c "import yaml;d=yaml.safe_load(open('.github/workflows/codex-loop-reusable.yml'));import json;print(json.dumps(d['jobs'],default=str))" | rg -o "correctness|security|performance|test-coverage|domain" | sort -u
    Expected Result: 5축 전부 존재, actionlint 0 error
    Evidence: .omo/evidence/task-13-review-axes.txt

  Scenario: dry-run posting 차단 (negative)
    Tool: Bash (rg)
    Steps:
      1. rg -n "ENABLE_REVIEW_POSTING|--dry-run" .github/workflows/codex-loop-reusable.yml
    Expected Result: dry-run 경로에서 publish가 --dry-run/비활성
    Evidence: .omo/evidence/task-13-dryrun-posting.txt
  ```

  **Commit**: YES (PR5)
  - Message: `feat(codex-loop): wire review stage (5-axis matrix, dry-run)`
  - Files: `.github/workflows/codex-loop-reusable.yml`
  - Pre-commit: `actionlint .github/workflows/codex-loop-reusable.yml`

- [x] 14. [HSI] design 스테이지 내부 matrix(cluster analysis) 결선

  **What to do**:
  - `stage=design` job 그룹에 design 파이프라인 결선: design_context → prepare_clusters(inventory/cluster) →
    analyze_clusters(matrix) → draft_plan → chief_decision → publish/route. 입력은 review state-bundle(`state_run_id`).
  - 산출 design-plan/chief-decision을 state-bundle로 업로드(fix 단계가 소비). dry-run에서 posting 비활성.

  **Must NOT do**:
  - review 산출을 head_sha 검색으로 취득 금지(`state_run_id` 사용). 단일 프롬프트 축약 금지.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — 기존 design 로직 결선.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 3 (with 13)
  - **Blocks**: 19
  - **Blocked By**: 8, 12

  **References**:
  - `rs-builder.../.github/workflows/codex-design.yml:131-513`(design_context/prepare_clusters(inventory+cluster)/analyze_clusters(matrix:261-322)/draft_plan/chief_decision/publish).
  - `setup/codex-review/src/codex_review/stages/{design,design_chief}` + `schemas/design-*.v1.schema.json` + `prompts/{design,design_chief}`.

  **Acceptance Criteria**:
  - [ ] design job이 inventory→cluster→analyze(matrix)→plan→chief→route 포함
  - [ ] review state-bundle을 `state_run_id`로 취득
  - [ ] dry-run 산출 + `actionlint` 0 error

  **QA Scenarios**:
  ```
  Scenario: design 파이프라인 결선 (happy)
    Tool: Bash (actionlint/rg)
    Steps:
      1. actionlint .github/workflows/codex-loop-reusable.yml
      2. rg -n "design inventory|design cluster|analyze|design-plan|chief" .github/workflows/codex-loop-reusable.yml | head
    Expected Result: design 단계 전부 결선, actionlint 0 error
    Evidence: .omo/evidence/task-14-design.txt

  Scenario: state_run_id 입력 사용 (negative-guard)
    Tool: Bash (rg)
    Steps:
      1. rg -n "headSha|run list" .github/workflows/codex-loop-reusable.yml
    Expected Result: design 취득에 headSha 검색 0
    Evidence: .omo/evidence/task-14-no-headsha.txt
  ```

  **Commit**: YES (PR5)
  - Message: `feat(codex-loop): wire design stage (cluster matrix, dry-run)`
  - Files: `.github/workflows/codex-loop-reusable.yml`
  - Pre-commit: `actionlint .github/workflows/codex-loop-reusable.yml`

- [x] 15. [HSI] fix 스테이지 내부 matrix(agents + merge + semantic-safety) 결선 (dry-run, no push)

  **What to do**:
  - `stage=fix` job 그룹에 fix 파이프라인 결선: plan_tasks(fix_dispatch) → run_agents(matrix) →
    merge_validate(premerge/merge model/semantic-safety/validate-fix/check-loop-budget). **이 태스크에서는 push 없음(dry-run)**;
    validated-fix + merged-fix를 state-bundle로 산출만. design state-bundle(`state_run_id`)을 입력으로.
  - 실제 commit/push 및 redispatch는 Wave 4(20–23)에서 결선(여기선 자리/출력 계약만).

  **Must NOT do**:
  - 이 태스크에서 commit/push/dispatch 금지(Wave 4). PR head를 secrets 하 실행 금지. fork에서 fix 산출을 push 가능 상태로 두지 않음.

  **Recommended Agent Profile**:
  - **Category**: `deep` — fix 파이프라인은 가장 복잡(merge/semantic-safety/budget) + 이후 push 계약의 토대.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: NO — Wave 3c (sub-wave, 14의 design 산출 shape에 의존하므로 13/14 이후)
  - **Blocks**: 19, 20
  - **Blocked By**: 8, 12, 14

  **References**:
  - `rs-builder.../.github/workflows/codex-fix.yml:135-349`(plan_tasks/run_agents(matrix:174-230)/merge_validate(premerge/semantic-safety/validate-fix/check-loop-budget)).
  - `setup/codex-review/src/codex_review/stages/{fix_dispatch,fix_merge,push}` + `schemas/fix-*.v1.schema.json` + `prompts/{fix_dispatch,fix_merge,push}`.
  - `setup/codex-review/src/codex_review/stages/push/orchestrate.py`(2-phase: validate 무토큰 단계).

  **Acceptance Criteria**:
  - [ ] fix job이 dispatch→agents(matrix)→merge→semantic-safety→validate→budget 포함(push 제외)
  - [ ] validated-fix/merged-fix가 state-bundle로 산출
  - [ ] dry-run에서 push/commit 단계 비실행 + `actionlint` 0 error

  **QA Scenarios**:
  ```
  Scenario: fix 파이프라인(무push) 결선 (happy)
    Tool: Bash (actionlint/rg)
    Steps:
      1. actionlint .github/workflows/codex-loop-reusable.yml
      2. rg -n "fix_dispatch|run_agents|fix_merge|semantic|validate-fix|check-loop-budget" .github/workflows/codex-loop-reusable.yml | head
    Expected Result: 단계 결선 존재, actionlint 0 error
    Evidence: .omo/evidence/task-15-fix.txt

  Scenario: dry-run push 차단 (negative)
    Tool: Bash (rg)
    Steps:
      1. rg -n "commit-push|git push|record-push" .github/workflows/codex-loop-reusable.yml
    Expected Result: dry-run/Wave3 골격에서 push 호출 0 (또는 dry_run==false 가드 뒤)
    Evidence: .omo/evidence/task-15-no-push-error.txt
  ```

  **Commit**: YES (PR5)
  - Message: `feat(codex-loop): wire fix stage (agents+merge+safety, dry-run, no push)`
  - Files: `.github/workflows/codex-loop-reusable.yml`
  - Pre-commit: `actionlint .github/workflows/codex-loop-reusable.yml`

- [x] 16. [HSI] issue 스테이지 결선 (MVP: terminal artifact/summary only) [DECISION]

  **What to do**:
  - `stage=issue` job 그룹을 결선하되, **MVP 기본값**: GitHub 이슈를 실제 생성하지 않고 `issue_fallback` 산출(reason+plan+content)을
    **terminal artifact + job summary**로만 남긴다(PR#18 non-goal "no issue creation/permission" 준수).
  - `[DECISION NEEDED]`: 실제 이슈 생성/`issues: write` 권한을 MVP에 포함할지. 기본은 deferred(artifact-only).
    사용자가 "포함"을 택하면 `issue_fallback apply` + App 토큰 issues:write로 확장(별도 task로 분기).

  **Must NOT do**:
  - 기본 MVP에서 `issues: write` 권한 추가/이슈 생성 금지(결정 전까지). 이슈를 루프 상태로 사용 금지.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low` — MVP는 산출/요약 한정(저복잡), 결정 시 확장.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 3
  - **Blocks**: 25
  - **Blocked By**: 12

  **References**:
  - `rs-builder.../.github/workflows/codex-issue.yml:27-207`(compose_content/publish_issue + needs-issue→issue-created 라벨 스왑).
  - `setup/codex-review/src/codex_review/stages/issue_fallback` + `schemas/issue-fallback-content.v1.schema.json`.
  - `home-server-infra/docs/codex-loop-reusable.md` Non-Goals("no issue creation/permission yet").

  **Acceptance Criteria**:
  - [ ] issue job이 reason/plan/content를 산출해 terminal artifact + summary로 기록(이슈 미생성, 기본값)
  - [ ] 워크플로우에 `issues: write` 권한 부재(MVP 기본)
  - [ ] 결정이 "포함"이면: 별도 task로 issues:write + apply 분기(문서화)

  **QA Scenarios**:
  ```
  Scenario: issue MVP terminal 산출 (happy)
    Tool: Bash (actionlint/rg)
    Steps:
      1. actionlint .github/workflows/codex-loop-reusable.yml
      2. rg -n "issues: write" .github/workflows/codex-loop-reusable.yml || echo "no issues:write (expected MVP)"
    Expected Result: issue 산출 결선, issues:write 부재
    Evidence: .omo/evidence/task-16-issue.txt

  Scenario: 이슈를 상태로 쓰지 않음 (negative)
    Tool: Bash (rg)
    Steps:
      1. rg -n "issue-created|needs-issue" .github/workflows/codex-loop-reusable.yml
    Expected Result: 라벨/이슈 상태 스왑 매치 0
    Evidence: .omo/evidence/task-16-no-issue-state.txt
  ```

  **Commit**: YES (PR5)
  - Message: `feat(codex-loop): wire issue stage (terminal artifact only, MVP)`
  - Files: `.github/workflows/codex-loop-reusable.yml`
  - Pre-commit: `actionlint .github/workflows/codex-loop-reusable.yml`

- [x] 17. [HSI] `finalize-stage` 정규화기 (outputs + state pointers + `updated_head_sha` placeholder)

  **What to do**:
  - 모든 stage 산출을 공통 출력으로 정규화하는 `finalize-stage` job:
    `{next_stage, lgtm, should_redispatch, terminal_reason, state_run_id, state_artifact_name, updated_head_sha}`.
  - terminal_reason은 task 3 taxonomy 사용. dry-run에서는 should_redispatch=false 강제 + terminal=dry_run.
    `updated_head_sha`는 placeholder(실제 캡처는 21). 출력 한계 대비: 대용량은 artifact, 포인터만 outputs.

  **Must NOT do**:
  - 모델 대용량 출력을 job outputs로 직접 전달 금지(artifact + 포인터). 라벨/코멘트로 상태 산출 금지.

  **Recommended Agent Profile**:
  - **Category**: `deep` — 루프 제어의 출력 계약(redispatch/terminal 판단 근거).
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: NO — Wave 3d (13–16 stage 산출 형태를 통합 정규화하므로 그 이후)
  - **Blocks**: 20, 21, 22
  - **Blocked By**: 2, 3, 9, 12, 13, 14, 15, 16

  **References**:
  - `home-server-infra/.github/workflows/codex-loop-reusable.yml`(finalize job: next_stage/lgtm/should_redispatch/terminal_reason 패턴, ~run-stage/parse/finalize).
  - task 2(state pointers), task 3(taxonomy), task 9(artifact 상태).
  - 기존 라우팅: `codex-review.yml:492-498`(loop route-after-techlead), `codex-fix.yml:338-339`(validation outputs).

  **Acceptance Criteria**:
  - [ ] finalize-stage가 7개 출력 키 전부 정규화 산출
  - [ ] dry-run에서 should_redispatch=false + terminal_reason=dry_run
  - [ ] terminal_reason 값이 taxonomy enum에 속함(검증 step)

  **QA Scenarios**:
  ```
  Scenario: 출력 정규화 (happy)
    Tool: Bash (actionlint/rg)
    Steps:
      1. actionlint .github/workflows/codex-loop-reusable.yml
      2. for k in next_stage lgtm should_redispatch terminal_reason state_run_id state_artifact_name updated_head_sha; do rg -q "$k" .github/workflows/codex-loop-reusable.yml || echo "MISSING $k"; done
    Expected Result: MISSING 없음, actionlint 0 error
    Evidence: .omo/evidence/task-17-finalize.txt

  Scenario: dry-run redispatch 차단 (negative)
    Tool: Bash (rg)
    Steps:
      1. rg -n "dry_run.*should_redispatch|should_redispatch.*false" .github/workflows/codex-loop-reusable.yml
    Expected Result: dry-run에서 redispatch 강제 false
    Evidence: .omo/evidence/task-17-dryrun-redispatch.txt
  ```

  **Commit**: YES (PR5)
  - Message: `feat(codex-loop): finalize-stage output normalizer`
  - Files: `.github/workflows/codex-loop-reusable.yml`
  - Pre-commit: `actionlint .github/workflows/codex-loop-reusable.yml`

- [x] 18. [HSI] state bundle artifact 업/다운로드 by explicit `run_id`

  **What to do**:
  - stage 간 state-bundle을 표준화: 각 stage가 `state_artifact_name`으로 bundle 업로드, 다음 stage가 payload의
    `state_run_id`로 `download-artifact run-id` 명시 취득. bundle 스키마(필수 파일 목록)와 무결성 체크 포함.
  - bundle 내용 계약: review→techlead 산출/design-plan/chief/fix manifest/validated-fix/loop history/`dispatch-ledger.json`.

  **Must NOT do**:
  - head_sha 검색/암묵 최신 run 사용 금지. bundle에 secret/모델 raw(민감) 포함 금지. 누락 bundle을 성공으로 간주 금지.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — 상태 전송 신뢰성.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 3
  - **Blocks**: 19, 22
  - **Blocked By**: 2, 9, 10

  **References**:
  - `rs-builder.../.github/workflows/codex-design.yml:98-129`(download-artifact `run-id` 사용 패턴), `codex-fix.yml:103-124`.
  - task 2(state pointers), task 10(state_run_id 전송), Oracle Part B-2.

  **Acceptance Criteria**:
  - [ ] 업/다운로드가 `state_artifact_name` + `run-id`(명시) 사용
  - [ ] bundle 무결성 체크(필수 파일 부재 시 `artifact_missing` terminal)
  - [ ] 단위/픽스처 테스트로 bundle round-trip 검증

  **QA Scenarios**:
  ```
  Scenario: bundle round-trip (happy)
    Tool: Bash (pytest/rg)
    Steps:
      1. uvx pytest setup/codex-review/tests -q -k "bundle or artifact"
      2. rg -n "run-id|state_artifact_name" .github/workflows/codex-loop-reusable.yml | head
    Expected Result: 테스트 통과, 명시 run-id 사용
    Evidence: .omo/evidence/task-18-bundle.txt

  Scenario: 누락 bundle terminal (negative)
    Tool: Bash (pytest -k)
    Steps:
      1. uvx pytest setup/codex-review/tests -q -k "artifact_missing or missing_bundle"
    Expected Result: 누락 시 artifact_missing terminal 검증 통과
    Evidence: .omo/evidence/task-18-missing.txt
  ```

  **Commit**: YES (PR5)
  - Message: `feat(codex-loop): explicit run_id state bundle transport`
  - Files: `.github/workflows/codex-loop-reusable.yml`, `setup/codex-review/src/codex_review/**`, tests
  - Pre-commit: `uvx pytest setup/codex-review/tests -q -k bundle && actionlint .github/workflows/codex-loop-reusable.yml`

- [x] 19. [HSI] 단계별 CLI/action parity 테스트 (fixture → 동일 schema shape)

  **What to do**:
  - 구식 4-워크플로우 단계 출력과 새 단일 reusable 단계 출력이 **동일 schema shape**임을 보장하는 parity 테스트 추가:
    review/design/fix 각 단계 fixture 입력 → 새 stage runner 산출이 기존 `schemas/*.v1`에 부합. issue는 보존 또는 명시적 skip(문서화).

  **Must NOT do**:
  - parity를 맞추려 schema/prompt를 임의 변경 금지. 통과를 위한 fixture 약화 금지.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — 다운그레이드 아님을 증명하는 핵심 안전망.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: NO (Wave 3 마감 게이트 — 13/14/15/18 통합)
  - **Blocks**: Wave 4 게이트
  - **Blocked By**: 13, 14, 15, 18

  **References**:
  - `setup/codex-review/schemas/{review-axis-findings,techlead-decision,design-plan,design-chief-decision,fix-merge-merged-fix,push-validated-fix}.v1.schema.json`.
  - `setup/codex-review/tests/fixtures`(기존 픽스처), `tests/unit`(검증 패턴).
  - Oracle Part B-6 parity 요구.

  **Acceptance Criteria**:
  - [ ] review/design/fix 단계 parity 테스트가 기존 v1 schema에 부합함을 검증
  - [ ] `uvx pytest -k parity` 전부 통과
  - [ ] issue 보존/skip 결정이 테스트에 문서화

  **QA Scenarios**:
  ```
  Scenario: 단계 parity (happy)
    Tool: Bash (pytest)
    Steps:
      1. uvx pytest setup/codex-review/tests -q -k "parity"
    Expected Result: review/design/fix parity 전부 통과
    Evidence: .omo/evidence/task-19-parity.txt

  Scenario: schema 무단 변경 방어 (negative)
    Tool: Bash (git)
    Steps:
      1. git -C <hsi> diff --stat origin/main -- setup/codex-review/schemas
    Expected Result: parity 목적 외 schema 변경 0(변경 시 근거 필요)
    Evidence: .omo/evidence/task-19-schema-diff.txt
  ```

  **Commit**: YES (PR5)
  - Message: `test(codex-loop): stage parity vs legacy pipeline`
  - Files: `setup/codex-review/tests/**`
  - Pre-commit: `uvx pytest setup/codex-review/tests -q -k parity`

- [x] 20. [HSI] trusted 2-phase fix-push + stale-head 재검증

  **What to do**:
  - fix 단계의 commit/push를 2-phase로 결선: (1) 무토큰 검증/테스트(15 산출 validated-fix), (2) `pr-head-write/` 신규
    checkout(정확히 `head_sha`) → 검증된 patch 적용 → commit → push. push 직전 **현재 PR head == payload head_sha** 재검증
    (불일치 시 `stale_head` terminal). same-repo + 비-fork만 push.
  - push에는 GitHub App installation token 사용(스테이지별 mint). 변경 없으면 `no_fix_changes` terminal.

  **Must NOT do**:
  - PR head 트리에서 테스트/스크립트 실행 후 그대로 push 금지(pr-head는 데이터; push는 pr-head-write에서). fork push 금지. PAT 금지.

  **Recommended Agent Profile**:
  - **Category**: `deep` — write 경로 + 보안 가드, 최고 위험.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: NO (Wave 4 선행)
  - **Blocks**: 21, 24
  - **Blocked By**: 15, 17

  **References**:
  - `setup/codex-review/src/codex_review/stages/push/orchestrate.py:3,332`(2-phase: 무토큰 검증 → push 직전 head 재검증).
  - `rs-builder.../.github/workflows/codex-fix.yml:352-422`(commit_push job: app_token mint → push → record).
  - `rs-builder.../.github/workflows/codex-fix.yml:65-69`(fork 차단), task 4(pr-head-write 토폴로지).

  **Acceptance Criteria**:
  - [ ] push가 pr-head-write 신규 checkout + App token으로만 수행
  - [ ] push 직전 head 재검증(불일치 → `stale_head` terminal)
  - [ ] 변경 없음 → `no_fix_changes` terminal, fork → push 미수행

  **QA Scenarios**:
  ```
  Scenario: 2-phase push 가드 결선 (happy, dry-run-safe)
    Tool: Bash (actionlint/rg/pytest)
    Steps:
      1. actionlint .github/workflows/codex-loop-reusable.yml
      2. rg -n "pr-head-write|app-token|stale_head|no_fix_changes" .github/workflows/codex-loop-reusable.yml setup/codex-review/src | head
      3. uvx pytest setup/codex-review/tests -q -k "push and (stale or no_fix or two_phase)"
    Expected Result: 가드 결선 존재, 관련 테스트 통과
    Evidence: .omo/evidence/task-20-push.txt

  Scenario: stale head 차단 (negative)
    Tool: Bash (pytest -k)
    Steps:
      1. uvx pytest setup/codex-review/tests -q -k "stale_head"
    Expected Result: payload head != live head → push 거부 + stale_head terminal
    Evidence: .omo/evidence/task-20-stale.txt
  ```

  **Commit**: YES (PR6)
  - Message: `feat(codex-loop): trusted two-phase fix-push with stale-head guard`
  - Files: `.github/workflows/codex-loop-reusable.yml`, `setup/codex-review/src/codex_review/stages/push/**`, tests
  - Pre-commit: `uvx pytest setup/codex-review/tests -q -k push && actionlint .github/workflows/codex-loop-reusable.yml`

- [x] 21. [HSI] `updated_head_sha` 캡처 + 원격 검증 + output

  **What to do**:
  - push 성공 후 원격에서 새 head SHA를 검증 취득(예: `git rev-parse` 후 GitHub API로 PR head 재조회 일치 확인) →
    `updated_head_sha`를 finalize-stage 출력으로 승격. 검증 실패 시 `pushed_unverified` terminal.
  - 기존 push 산출의 `remote_head_sha`/`expected_head_sha`를 활용해 일치 확정.

  **Must NOT do**:
  - 검증 없이 로컬 SHA를 updated로 신뢰 금지(원격 확정 필요). 미검증 상태로 redispatch 금지.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — 연속 루프의 정확성(다음 review가 새 head를 봄).
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: NO (task 20 직후)
  - **Blocks**: 22
  - **Blocked By**: 20

  **References**:
  - `setup/codex-review/src/codex_review/stages/push/push.py:40`(remote_head_sha/expected_head_sha 보고).
  - `rs-builder.../.github/workflows/codex-fix.yml:400-417`(push_result/record-push 패턴), task 17(finalize 출력).

  **Acceptance Criteria**:
  - [ ] push 후 원격 head SHA 재검증 + `updated_head_sha` 출력
  - [ ] 불일치/미검증 → `pushed_unverified` terminal
  - [ ] 단위 테스트로 검증 경로 통과

  **QA Scenarios**:
  ```
  Scenario: updated head 검증 (happy)
    Tool: Bash (pytest/rg)
    Steps:
      1. uvx pytest setup/codex-review/tests -q -k "updated_head or remote_head"
      2. rg -n "updated_head_sha|pushed_unverified|remote_head_sha" .github/workflows/codex-loop-reusable.yml setup/codex-review/src | head
    Expected Result: 검증 경로 + 출력 존재, 테스트 통과
    Evidence: .omo/evidence/task-21-updated-head.txt

  Scenario: 미검증 push 차단 (negative)
    Tool: Bash (pytest -k)
    Steps:
      1. uvx pytest setup/codex-review/tests -q -k "pushed_unverified"
    Expected Result: 원격 불일치 시 pushed_unverified terminal
    Evidence: .omo/evidence/task-21-unverified.txt
  ```

  **Commit**: YES (PR6)
  - Message: `feat(codex-loop): verified updated_head_sha output`
  - Files: `setup/codex-review/src/codex_review/stages/push/**`, `.github/workflows/codex-loop-reusable.yml`, tests
  - Pre-commit: `uvx pytest setup/codex-review/tests -q -k updated_head`

- [x] 22. [HSI] `repository_dispatch` 연속 emit (App token) + 최소 권한

  **What to do**:
  - finalize에서 `should_redispatch=true`(LGTM 아님 + 진행)일 때, **App installation token**으로
    `repos/:owner/:repo/dispatches`(`event_type=codex-loop`) 전송. payload: next_stage, `head_sha`(fix면 `updated_head_sha`),
    base_ref, iteration+1, correlation_id, state_run_id(=현재 run), state_artifact_name, requested_by, dry_run, max_iterations, schema_version.
  - 최소 권한: 연속 dispatch=`contents: write`, PR 재검증=`pull-requests: read`. 스테이지별 토큰 mint(만료 회피).
  - PR#18의 `trusted-fix-push-not-implemented` 종료 분기를 실제 연속 emit으로 대체(live 한정; dry-run은 emit 안 함).

  **Must NOT do**:
  - GITHUB_TOKEN/PAT로 dispatch 금지(App token만). dry-run에서 실제 dispatch 금지. payload에 secret/모델 raw 금지.

  **Recommended Agent Profile**:
  - **Category**: `deep` — self-driving 루프의 심장(연속 트리거 + 권한).
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: NO (Wave 4 핵심)
  - **Blocks**: 23, 27
  - **Blocked By**: 5, 10, 17, 18, 21

  **References**:
  - `home-server-infra/.github/workflows/codex-loop-reusable.yml`(finalize의 `trusted-fix-push-not-implemented` 분기 — 대체 대상).
  - `home-server-infra/.github/workflows/codex-loop-dispatch.yml`(dispatch payload 검증/매핑 — emit 형태 정합).
  - `rs-builder.../.github/workflows/codex-fix.yml:386-394`(App token mint `auth app-token` 패턴), `codex-review.yml:529`(App token이 다음 워크플로우 재트리거하는 이유).
  - Oracle Part B-3(연속 권한/디폴트브랜치 제약).

  **Acceptance Criteria**:
  - [ ] live + should_redispatch → App token으로 dispatch emit(payload 키 완비)
  - [ ] fix 연속은 `updated_head_sha` 사용 + iteration+1
  - [ ] dry-run에서 emit 미수행, GITHUB_TOKEN/PAT dispatch 부재

  **QA Scenarios**:
  ```
  Scenario: 연속 dispatch emit 결선 (happy, dry-run-safe 검사)
    Tool: Bash (actionlint/rg)
    Steps:
      1. actionlint .github/workflows/codex-loop-reusable.yml
      2. rg -n "dispatches|event_type=codex-loop|app.*token|iteration.*\+.*1|updated_head_sha" .github/workflows/codex-loop-reusable.yml | head
    Expected Result: App-token dispatch emit + iteration+1 + updated_head_sha 결선
    Evidence: .omo/evidence/task-22-dispatch.txt

  Scenario: dry-run/비-App dispatch 차단 (negative)
    Tool: Bash (rg)
    Steps:
      1. rg -n "GITHUB_TOKEN.*dispatches|github.token.*dispatch" .github/workflows/codex-loop-reusable.yml
      2. rg -n "dry_run.*dispatch|if:.*dry_run == false" .github/workflows/codex-loop-reusable.yml
    Expected Result: GITHUB_TOKEN dispatch 0, dispatch가 dry_run==false 가드 뒤
    Evidence: .omo/evidence/task-22-guard.txt
  ```

  **Commit**: YES (PR6)
  - Message: `feat(codex-loop): App-token repository_dispatch continuation`
  - Files: `.github/workflows/codex-loop-reusable.yml`
  - Pre-commit: `actionlint .github/workflows/codex-loop-reusable.yml`

- [x] 23. [HSI] dispatch ledger + caps(iteration/per-correlation) + concurrency 통일

  **What to do**:
  - runaway 방지: state-bundle에 `dispatch-ledger.json` 유지 — 동일 `{correlation_id, stage, iteration, head_sha}`가 이미
    emit됐으면 `dispatch_duplicate` terminal로 거부. per-correlation 누적 dispatch 상한 강제.
  - iteration cap(`>= max_iterations` → `max_iterations` terminal, task 5와 일관), oscillation 감지(동일 실패 시그니처 반복 →
    `oscillation_detected`). core/dispatch concurrency group을 `correlation_id` 기준으로 최종 통일.

  **Must NOT do**:
  - cap 없이 무한 redispatch 허용 금지. ledger를 라벨/코멘트로 저장 금지(artifact). dry-run/live concurrency 공유 충돌 금지.

  **Recommended Agent Profile**:
  - **Category**: `deep` — 루프 안전성의 마지막 방어선.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 4 (task 22 이후)
  - **Blocks**: 26
  - **Blocked By**: 5, 22

  **References**:
  - task 5(iteration `>=`/concurrency), task 18(state-bundle), Metis "dispatch storm/runaway" mitigation, Oracle Part B-3 guardrails 3.
  - `home-server-infra/.github/workflows/codex-loop-reusable.yml:67`, `codex-loop-dispatch.yml:9`(concurrency 키).

  **Acceptance Criteria**:
  - [ ] dispatch ledger가 중복 emit을 `dispatch_duplicate`로 거부
  - [ ] iteration cap + oscillation 감지 terminal 동작
  - [ ] concurrency group이 두 워크플로우에서 correlation_id 기준 일치

  **QA Scenarios**:
  ```
  Scenario: 중복/상한 차단 (happy/negative)
    Tool: Bash (pytest/rg)
    Steps:
      1. uvx pytest setup/codex-review/tests -q -k "ledger or duplicate or oscillation or max_iterations"
      2. rg -n "dispatch-ledger|dispatch_duplicate|oscillation_detected" .github/workflows setup/codex-review/src | head
    Expected Result: 중복/상한/진동 terminal 검증 통과
    Evidence: .omo/evidence/task-23-ledger.txt

  Scenario: concurrency 통일 (negative-guard)
    Tool: Bash (rg)
    Steps:
      1. rg -nA1 "concurrency:" .github/workflows/codex-loop-reusable.yml .github/workflows/codex-loop-dispatch.yml
    Expected Result: group이 correlation_id 기준 정합
    Evidence: .omo/evidence/task-23-concurrency.txt
  ```

  **Commit**: YES (PR6)
  - Message: `feat(codex-loop): dispatch ledger + loop caps + unified concurrency`
  - Files: `.github/workflows/codex-loop-*.yml`, `setup/codex-review/src/codex_review/**`, tests
  - Pre-commit: `uvx pytest setup/codex-review/tests -q -k ledger && actionlint .github/workflows/codex-loop-*.yml`

- [x] 24. [HSI] fork/trust boundary 강제 (no fix-push, no secrets on untrusted)

  **What to do**:
  - trust-and-stale-guard를 강화: PR owner/repo/fork 여부/actor/base_ref 검증. fork PR이면 relay/secret/write 이전에
    종료(`fork_pr`/`untrusted_repository_owner`). fork는 fix-push/연속 dispatch 비활성(review만 허용 여부는 정책에 따라, 기본 종료).
  - `requested_by` 스푸핑 방지(payload requested_by를 신뢰된 게이트에서만 사용). PR head는 데이터 전용 재확인.

  **Must NOT do**:
  - fork PR에 secrets/OIDC/write 노출 금지. payload requested_by를 권한 판단의 단독 근거로 신뢰 금지.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — 보안 경계 강제.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 4 (task 20 이후)
  - **Blocks**: 26
  - **Blocked By**: 4, 20

  **References**:
  - `rs-builder.../.github/workflows/codex-fix.yml:65-69`(fork 차단), `codex-review.yml:68-78`(actor 권한 검사).
  - `home-server-infra/.github/workflows/codex-loop-reusable.yml:~202`(headRefOid vs head_sha), task 4(trust boundary).
  - Metis/Oracle fork·trust 가드.

  **Acceptance Criteria**:
  - [ ] fork PR이 relay/secret/write 이전에 `fork_pr`/`untrusted_repository_owner` terminal
  - [ ] fork에서 fix-push/연속 dispatch 미수행
  - [ ] requested_by 단독 신뢰 부재(게이트 검증 병행)

  **QA Scenarios**:
  ```
  Scenario: fork 종료 (negative, 핵심)
    Tool: Bash (pytest/rg)
    Steps:
      1. uvx pytest setup/codex-review/tests -q -k "fork or untrusted"
      2. rg -n "fork|head.repo.full_name|owner|fork_pr" .github/workflows/codex-loop-reusable.yml | head
    Expected Result: fork → 조기 terminal, secrets 미노출
    Evidence: .omo/evidence/task-24-fork.txt

  Scenario: same-repo 정상 통과 (happy)
    Tool: Bash (pytest -k)
    Steps:
      1. uvx pytest setup/codex-review/tests -q -k "same_repo or trusted"
    Expected Result: same-repo trusted PR은 통과
    Evidence: .omo/evidence/task-24-samerepo.txt
  ```

  **Commit**: YES (PR6)
  - Message: `feat(codex-loop): enforce fork/trust boundary`
  - Files: `.github/workflows/codex-loop-reusable.yml`, `setup/codex-review/src/codex_review/**`, tests
  - Pre-commit: `uvx pytest setup/codex-review/tests -q -k fork && actionlint .github/workflows/codex-loop-reusable.yml`

- [x] 25. [HSI] 실패/종료 가시화 (job summary + artifacts + conclusion)

  **What to do**:
  - 모든 비-LGTM 종료가 라벨/이슈 없이도 사람에게 보이도록: `terminal_reason` + 머신리더블 `codex-loop-state.json`/
    `terminal-summary.md` artifact + `$GITHUB_STEP_SUMMARY` 휴먼 요약 + workflow conclusion(실패는 실패로). 다음 수동 조치/관련 artifact 링크 포함.
  - (선택, defer) Check Run: 권한/복잡도 고려해 live parity 증명 후로 연기. 본 태스크는 summary/artifact/conclusion 기준.

  **Must NOT do**:
  - 종료를 라벨/코멘트/이슈로만 표기 금지. 실패를 success conclusion으로 가리지 않음. secret을 summary/artifact에 노출 금지.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — 관찰가능성(Metis 최우선 리스크).
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 4
  - **Blocks**: 26
  - **Blocked By**: 3, 16

  **References**:
  - `home-server-infra/docs/codex-loop-reusable.md`("Terminal failures ... job summary and artifacts").
  - `home-server-infra/.github/workflows/codex-loop-reusable.yml`(GITHUB_STEP_SUMMARY 작성 패턴), task 3(taxonomy).
  - Metis "invisible state machine" mitigation.

  **Acceptance Criteria**:
  - [ ] 각 종료가 terminal-summary.md + codex-loop-state.json artifact + step summary 산출
  - [ ] 실패 종료는 workflow conclusion=failure(또는 명시적 neutral), success로 위장 안 함
  - [ ] summary에 다음 조치/artifact 링크 포함

  **QA Scenarios**:
  ```
  Scenario: 종료 가시화 산출 (happy)
    Tool: Bash (rg)
    Steps:
      1. rg -n "GITHUB_STEP_SUMMARY|terminal-summary|codex-loop-state.json|upload-artifact" .github/workflows/codex-loop-reusable.yml | head
    Expected Result: summary+artifact 산출 결선
    Evidence: .omo/evidence/task-25-visibility.txt

  Scenario: 실패 위장 방어 (negative)
    Tool: Bash (rg)
    Steps:
      1. rg -n "continue-on-error: true|exit 0" .github/workflows/codex-loop-reusable.yml
    Expected Result: terminal 실패가 success로 가려지지 않음(부적절한 continue-on-error 없음)
    Evidence: .omo/evidence/task-25-no-mask.txt
  ```

  **Commit**: YES (PR6)
  - Message: `feat(codex-loop): label-free terminal visibility`
  - Files: `.github/workflows/codex-loop-reusable.yml`
  - Pre-commit: `actionlint .github/workflows/codex-loop-reusable.yml`

- [x] 26. [HSI] negative runtime + dry-run dispatch smoke 증거

  **What to do**:
  - dry-run `repository_dispatch` smoke(테스트 PR): `gh api repos/<owner>/<repo>/dispatches event_type=codex-loop dry_run=true` →
    HTTP 204 → run 시작 → 결정론적 stage 결과 artifact + outputs(next_stage/should_redispatch/terminal_reason) 확인.
  - negative runtime 검증(dry-run-safe 또는 단위): stale SHA/max iteration/fork/closed PR/missing App creds/relay outage가
    각각 올바른 terminal로 종료(silent loop 아님). 증거 수집(`gh run view --json conclusion,jobs`).

  **Must NOT do**:
  - 승인 없이 live(non-dry-run) dispatch 금지. 모호한 상태를 성공으로 간주 금지. 증거 없이 "동작" 주장 금지.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — Wave 4 마감 검증(증거 기반).
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: NO (Wave 4 마감 게이트 — 22/23/24/25 통합)
  - **Blocks**: Wave 5 게이트
  - **Blocked By**: 22, 23, 24, 25

  **References**:
  - `home-server-infra/docs/codex-loop-reusable.md` "Default-Branch Dispatch Smoke Test"(gh api dispatches + gh run list/view 절차).
  - task 6(payload neg 테스트), task 23(caps), Metis negative runtime 목록.

  **Acceptance Criteria**:
  - [ ] dry-run dispatch가 HTTP 204 + 성공 run + 결정론적 결과 artifact(증거 저장)
  - [ ] stale/max-iter/fork/closed-PR/missing-creds가 각각 올바른 terminal_reason(증거)
  - [ ] live(non-dry-run)는 본 태스크에서 실행하지 않음(승인 게이트)

  **QA Scenarios**:
  ```
  Scenario: dry-run dispatch smoke (happy)
    Tool: Bash (gh)
    Preconditions: HSI default branch에 codex-loop-dispatch.yml 존재 + 테스트 PR
    Steps:
      1. gh api repos/<owner>/<repo>/dispatches --method POST -f event_type=codex-loop -F 'client_payload[stage]=review' -F 'client_payload[dry_run]=true' -F 'client_payload[pr_number]=<n>' -F 'client_payload[head_sha]=<sha>' -F 'client_payload[base_ref]=main' -F 'client_payload[iteration]=0' -F 'client_payload[correlation_id]=dryrun-<n>' -F 'client_payload[requested_by]=manual'
      2. sleep 15; gh run list --repo <owner>/<repo> --workflow codex-loop-dispatch.yml --json databaseId,status,conclusion --limit 3
      3. gh run view <id> --repo <owner>/<repo> --json conclusion,jobs | tee .omo/evidence/task-26-smoke.json
    Expected Result: 204 → run success → 결정론적 dry-run 결과, 라벨/코멘트 변동 없음
    Evidence: .omo/evidence/task-26-smoke.json

  Scenario: negative terminal 경로 (negative)
    Tool: Bash (pytest/gh)
    Steps:
      1. uvx pytest setup/codex-review/tests -q -k "stale or max_iteration or fork or pr_closed or missing_app"
    Expected Result: 각 케이스가 해당 terminal_reason로 종료(silent loop 0)
    Evidence: .omo/evidence/task-26-negative.txt
  ```

  **Commit**: YES (PR6)
  - Message: `test(codex-loop): dry-run dispatch smoke + negative runtime evidence`
  - Files: `tests/workflows/**`, `.omo/evidence/**`(증거)
  - Pre-commit: `uvx pytest tests/workflows -q`

- [x] 27. [RS] thin SHA-pinned `pull_request_target` 어댑터 (dry-run)

  **What to do**:
  - rs-builder에 얇은 consumer 어댑터 추가: `pull_request_target`(opened/synchronize/reopened/ready_for_review, draft 제외) →
    `uses: DongwonTTuna-Labs/home-server-infra/.github/workflows/codex-loop-reusable.yml@<task-1 SHA>` 호출.
    `stage: review`, `iteration: 0`, `correlation_id: codex-loop-${pr}-${head_sha}-0`, `dry_run: true`, `secrets: inherit`.
  - 권한은 ceiling 최소(contents:write, pull-requests:write, id-token:write). SHA pin 필수(branch pin 금지, AGENTS.md).

  **Must NOT do**:
  - branch/tag(moving ref) pin 금지(commit SHA만). 초기엔 dry_run=false 금지. 라벨 트리거 추가 금지.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — consumer 진입점(보안/핀 정확성).
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: NO — Wave 5a (Wave 4 게이트 task 26 이후 시작)
  - **Blocks**: 28, 29
  - **Blocked By**: 8, 22, 26

  **References**:
  - `home-server-infra/docs/codex-loop-reusable.md` "Org Consumer Adapters"(샘플 pull_request_target 어댑터, SHA pin, dry_run:true).
  - task 1 SHA(`.omo/evidence/task-1-merge-sha.txt`), AGENTS.md(RS) SHA pin 정책.
  - `rs-builder.../.github/workflows/codex-review.yml:9-21`(기존 pull_request_target/concurrency 참고, 단 라벨 트리거는 제거).

  **Acceptance Criteria**:
  - [ ] 어댑터가 reusable core를 **40자 SHA**로 pin해 호출(`@<sha>`)
  - [ ] `dry_run: true`, draft PR 제외, 최소 권한 ceiling
  - [ ] `actionlint` 0 error, 라벨 트리거 부재

  **QA Scenarios**:
  ```
  Scenario: SHA-pinned dry-run 어댑터 (happy)
    Tool: Bash (actionlint/rg)
    Steps:
      1. actionlint .github/workflows/*.yml
      2. rg -n "home-server-infra/.github/workflows/codex-loop-reusable.yml@[0-9a-f]{40}" .github/workflows/
      3. rg -n "dry_run: true|draft == false" .github/workflows/
    Expected Result: 40-char SHA pin + dry_run:true + draft 제외, actionlint 0 error
    Evidence: .omo/evidence/task-27-adapter.txt

  Scenario: branch pin 회귀 방어 (negative)
    Tool: Bash (rg)
    Steps:
      1. rg -n "codex-loop-reusable.yml@(main|work/|v[0-9])" .github/workflows/
    Expected Result: moving-ref pin 매치 0
    Evidence: .omo/evidence/task-27-no-branch-pin.txt
  ```

  **Commit**: YES (PR7)
  - Message: `ci(codex-loop): add SHA-pinned dry-run pull_request_target adapter`
  - Files: `.github/workflows/codex-loop-review-adapter.yml`
  - Pre-commit: `actionlint .github/workflows/*.yml`

- [x] 28. [RS] manual `workflow_dispatch` + `repository_dispatch` 어댑터 (default branch)

  **What to do**:
  - manual 디버그 어댑터(`workflow_dispatch`, dry_run 기본 true)와 연속 수신용 `repository_dispatch`(types:[codex-loop]) 어댑터를
    rs-builder에 추가, 둘 다 reusable core를 동일 SHA로 호출. repository_dispatch는 **default branch에 존재**해야 발화(머지 필요).
  - dispatch 어댑터는 payload 검증 후 core 호출(home-server-infra dispatch 어댑터 계약과 정합).

  **Must NOT do**:
  - dispatch 어댑터를 비-default 브랜치에만 두기 금지(발화 안 함). manual 기본 dry_run=false 금지. SHA pin 누락 금지.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — 연속 루프 수신점.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: NO (task 27 직후)
  - **Blocks**: 29
  - **Blocked By**: 27

  **References**:
  - `home-server-infra/.github/workflows/codex-loop-{dispatch,manual}.yml`(어댑터 형태/payload 매핑).
  - `home-server-infra/docs/codex-loop-reusable.md` Rollout("merge dispatch adapter to default branch before smoke").
  - task 27(SHA pin 패턴).

  **Acceptance Criteria**:
  - [ ] manual + repository_dispatch 어댑터가 동일 SHA로 core 호출
  - [ ] repository_dispatch 어댑터가 default branch 병합 대상(문서/주석 명시)
  - [ ] `actionlint` 0 error, manual dry_run 기본 true

  **QA Scenarios**:
  ```
  Scenario: 두 어댑터 결선 (happy)
    Tool: Bash (actionlint/rg)
    Steps:
      1. actionlint .github/workflows/*.yml
      2. rg -n "repository_dispatch|workflow_dispatch|types: \[codex-loop\]|@[0-9a-f]{40}" .github/workflows/ | head
    Expected Result: 두 트리거 + SHA pin 존재, actionlint 0 error
    Evidence: .omo/evidence/task-28-adapters.txt

  Scenario: manual 기본 dry-run (negative-guard)
    Tool: Bash (rg)
    Steps:
      1. rg -nA3 "workflow_dispatch" .github/workflows/ | rg "dry_run|default: true"
    Expected Result: manual dry_run 기본 true
    Evidence: .omo/evidence/task-28-manual-dryrun.txt
  ```

  **Commit**: YES (PR7)
  - Message: `ci(codex-loop): add manual + repository_dispatch consumer adapters`
  - Files: `.github/workflows/codex-loop-{manual,dispatch}-adapter.yml`
  - Pre-commit: `actionlint .github/workflows/*.yml`

- [~] 29. [RS] 구식 라벨 워크플로우 비활성(중첩 방지) + dry-run/1회 live parity 증명

  **What to do**:
  - 신·구 시스템이 같은 PR에서 동시 동작하지 않도록 구식 4 워크플로우를 일시 비활성(트리거 무력화 또는 repo 설정 disable).
  - dry-run 스모크 재확인 후, **승인된 1회 live same-repo 루프**를 실행해 review→…→fix-push→`updated_head_sha`→redispatch→
    LGTM **자동완주**를 증명(증거: 각 run conclusion/jobs/artifact + 최종 terminal=lgtm). 이 증거가 Wave 6 제거의 게이트.

  **Must NOT do**:
  - parity 증명 전 구식 워크플로우 **삭제** 금지(이 태스크는 비활성까지). 신·구 동시 구동 금지. 증거 없이 "자동완주 됨" 주장 금지.

  **Recommended Agent Profile**:
  - **Category**: `deep` — 신·구 전환 검증 + live 루프 증명(고위험/고판단).
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: NO (Wave 5 마감 게이트)
  - **Blocks**: 30, 31, 32
  - **Blocked By**: 27, 28

  **References**:
  - `rs-builder.../.github/workflows/codex-{review,design,fix,issue}.yml`(비활성 대상 트리거).
  - task 26(dry-run smoke 절차), `home-server-infra/docs/codex-loop-reusable.md` Rollout("keep old label workflows disabled or separate; don't run both").

  **Acceptance Criteria**:
  - [ ] 구식 4 워크플로우가 비활성(트리거 미발화) — 신·구 중첩 0
  - [ ] dry-run smoke 재확인 통과
  - [ ] 1회 live 루프가 LGTM까지 자동완주(증거: run 체인 + terminal=lgtm)

  **QA Scenarios**:
  ```
  Scenario: live 자동완주 증명 (happy, 승인 게이트)
    Tool: Bash (gh)
    Preconditions: 승인된 live 테스트 PR, 어댑터 default branch 병합
    Steps:
      1. (kickoff) PR open 또는 manual dispatch dry_run=false
      2. gh run list --repo <owner>/<repo> --json databaseId,event,headSha,conclusion --limit 20 | tee .omo/evidence/task-29-loop-runs.json
      3. 루프 체인(review→design→fix→review…)과 최종 terminal=lgtm 확인, head_sha 갱신 확인
    Expected Result: redispatch 체인으로 LGTM 자동완주, 라벨/코멘트 상태 변동 없음
    Evidence: .omo/evidence/task-29-loop-runs.json

  Scenario: 신·구 중첩 방어 (negative)
    Tool: Bash (rg/gh)
    Steps:
      1. rg -n "types: \[labeled\]|github.event.label.name" .github/workflows/codex-{review,design,fix,issue}.yml
      2. (테스트 PR에 라벨 부착해도 구식 워크플로우 미발화 확인)
    Expected Result: 구식 라벨 트리거 비활성, 동시 구동 0
    Evidence: .omo/evidence/task-29-no-overlap.txt
  ```

  **Commit**: YES (PR7)
  - Message: `ci(codex-loop): disable legacy label workflows; live loop parity evidence`
  - Files: `.github/workflows/codex-{review,design,fix,issue}.yml`(트리거 비활성), `.omo/evidence/**`
  - Pre-commit: `actionlint .github/workflows/*.yml`

- [~] 30. [RS] 4 라벨 워크플로우 + `setup-codex-review` action 제거

  **What to do**:
  - parity 증명(task 29) 후, rs-builder의 `.github/workflows/codex-{review,design,fix,issue}.yml` 4개와
    `.github/actions/setup-codex-review/` composite action을 제거.
  - 제거 후 `actionlint`가 남은 워크플로우(consumer 어댑터)만 대상으로 0 error인지 확인.

  **Must NOT do**:
  - task 29 parity 증거 없이 제거 금지. consumer 어댑터(27/28) 제거 금지. `.github/actionlint.yaml` 등 공유 설정 무단 삭제 금지.

  **Recommended Agent Profile**:
  - **Category**: `quick` — 파일 제거 + lint 확인(저위험, 단 게이트 준수).
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 6 (with 31, 32)
  - **Blocks**: F1
  - **Blocked By**: 29

  **References**:
  - `rs-builder.../.github/workflows/codex-{review,design,fix,issue}.yml`, `.github/actions/setup-codex-review/action.yml`.
  - task 29 증거(`.omo/evidence/task-29-loop-runs.json`), Oracle Part B-7 PR8(cleanup).

  **Acceptance Criteria**:
  - [ ] 4개 라벨 워크플로우 + setup-codex-review action 파일 부재
  - [ ] 남은 워크플로우 `actionlint` 0 error
  - [ ] consumer 어댑터(27/28)는 잔존

  **QA Scenarios**:
  ```
  Scenario: 구식 워크플로우 제거 (happy)
    Tool: Bash (ls/actionlint)
    Steps:
      1. ls .github/workflows/codex-review.yml 2>&1 | rg "No such file" && echo removed
      2. test ! -d .github/actions/setup-codex-review && echo "action removed"
      3. actionlint .github/workflows/*.yml
    Expected Result: 4 워크플로우 + action 제거, actionlint 0 error
    Evidence: .omo/evidence/task-30-removed.txt

  Scenario: 어댑터 보존 (negative-guard)
    Tool: Bash (rg)
    Steps:
      1. rg -l "codex-loop-reusable.yml@" .github/workflows/
    Expected Result: consumer 어댑터 파일 존재(삭제되지 않음)
    Evidence: .omo/evidence/task-30-adapters-kept.txt
  ```

  **Commit**: YES (PR8)
  - Message: `chore(codex): remove legacy label workflows + setup-codex-review action`
  - Files: `.github/workflows/codex-{review,design,fix,issue}.yml`(삭제), `.github/actions/setup-codex-review/`(삭제)
  - Pre-commit: `actionlint .github/workflows/*.yml`

- [~] 31. [RS] `setup/codex-review` 패키지 + 라벨 ops 제거

  **What to do**:
  - rs-builder의 `setup/codex-review/` 전체 디렉터리 제거(이미 home-server-infra로 이식·검증 완료).
  - 라벨 운영 잔재 제거: 라벨 정의/문서/스크립트에서 `리뷰중/리뷰완료/설계중/설계완료/수정중/수정완료/codex:lgtm/
    codex:needs-issue/codex:issue-created/codex:label-misuse` 의존 정리(루프 상태 의미로 사용하던 부분).

  **Must NOT do**:
  - home-server-infra 이식본 검증(task 11/19) 없이 제거 금지. relayer/Rust 소스나 무관 디렉터리 제거 금지.

  **Recommended Agent Profile**:
  - **Category**: `quick` — 디렉터리/잔재 제거.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 6
  - **Blocks**: F1
  - **Blocked By**: 29

  **References**:
  - `rs-builder.../setup/codex-review/`(제거 대상, 이식본=HSI).
  - 라벨 사용처: `codex-review.yml:79-83`, `codex-fix.yml:91-98`, `codex-design.yml:85-93`, `codex-issue.yml:206-207`.

  **Acceptance Criteria**:
  - [ ] `setup/codex-review/` 부재
  - [ ] repo 내 루프-상태 라벨 의존 잔재 0(`rg`로 확인)
  - [ ] `cargo`/Rust 빌드 영향 없음(무관 영역 무변경)

  **QA Scenarios**:
  ```
  Scenario: 패키지/라벨 제거 (happy)
    Tool: Bash (ls/rg)
    Steps:
      1. test ! -d setup/codex-review && echo "pkg removed"
      2. rg -n "리뷰중|리뷰완료|설계완료|수정중|codex:lgtm|codex:needs-issue" . -g '!.omo/**' | wc -l
    Expected Result: 패키지 부재, 루프-상태 라벨 잔재 0
    Evidence: .omo/evidence/task-31-removed.txt

  Scenario: 무관 영역 무변경 (negative)
    Tool: Bash (git)
    Steps:
      1. git diff --stat origin/main -- src/ examples/ Cargo.toml
    Expected Result: relayer/Rust 소스 변경 0
    Evidence: .omo/evidence/task-31-no-rust-change.txt
  ```

  **Commit**: YES (PR8)
  - Message: `chore(codex): remove setup/codex-review + loop-state label ops`
  - Files: `setup/codex-review/`(삭제), 라벨 관련 잔재
  - Pre-commit: `rg -c "리뷰중|codex:lgtm" . -g '!.omo/**' || true`

- [~] 32. [RS] repo 문서(README/docs) 새 consumer 모델로 갱신

  **What to do**:
  - rs-builder README 및 관련 docs에서 구식 라벨 기반 Codex 파이프라인 설명을 제거/대체하고, **org-reusable consumer 모델**
    (home-server-infra reusable core를 SHA pin으로 소비, repository_dispatch 자가구동, dry-run-first, 라벨 미사용)로 갱신.
  - SHA pin/롤백(어댑터 비활성·dispatch 중단·App 크리덴셜 회전) 안내를 consumer 관점으로 간단히 기술.

  **Must NOT do**:
  - 구식 라벨 워크플로우 설명 잔존 금지. relayer SDK 문서(README의 relayer 사용법 등) 무단 변경 금지.

  **Recommended Agent Profile**:
  - **Category**: `writing` — 문서 정합/명료성.
  - **Skills**: 없음.

  **Parallelization**:
  - **Can Run In Parallel**: YES — Wave 6
  - **Blocks**: F1
  - **Blocked By**: 29

  **References**:
  - `rs-builder.../README.md`(Codex 관련 언급), `docs/`(파이프라인/리뷰 관련 문서가 있으면).
  - `home-server-infra/docs/codex-loop-reusable.md`(consumer 어댑터/SHA pin/rollback 출처).

  **Acceptance Criteria**:
  - [ ] 문서에 구식 라벨 파이프라인 설명 부재, consumer 모델/SHA pin/롤백 설명 존재
  - [ ] relayer SDK 사용법 섹션 무변경(diff로 확인)
  - [ ] 링크/경로 유효(깨진 참조 0)

  **QA Scenarios**:
  ```
  Scenario: 문서 갱신 (happy)
    Tool: Bash (rg)
    Steps:
      1. rg -n "리뷰중|codex-review.yml|라벨 부착" README.md docs/ | wc -l
      2. rg -n "codex-loop-reusable|SHA pin|repository_dispatch|dry-run" README.md docs/ | head
    Expected Result: 구식 설명 0, consumer 모델 설명 존재
    Evidence: .omo/evidence/task-32-docs.txt

  Scenario: relayer 문서 무변경 (negative)
    Tool: Bash (git)
    Steps:
      1. git diff origin/main -- README.md | rg -n "RelayClient|operations::|AuthMethod" | head
    Expected Result: relayer SDK 사용법 라인 변경 없음(또는 무관)
    Evidence: .omo/evidence/task-32-relayer-doc.txt
  ```

  **Commit**: YES (PR8)
  - Message: `docs(codex): document org-reusable consumer model`
  - Files: `README.md`, `docs/**`(해당 시)
  - Pre-commit: `rg -c "리뷰중" README.md docs/ || true`

---

## Final Verification Wave (MANDATORY — after ALL implementation tasks)

> 4 review agents run in PARALLEL. ALL must APPROVE. Present consolidated results to user and get explicit "okay".
> **Do NOT auto-proceed.** Never mark F1–F4 checked before user's okay.

- [~] F1. **Plan Compliance Audit** — `oracle`
  플랜을 끝까지 읽고 각 "Must Have" 구현 존재 확인(파일 read, `actionlint`, `pytest`, dry-run dispatch),
  각 "Must NOT Have" 금지 패턴을 두 repo에서 검색해 위반 시 file:line으로 reject. `.omo/evidence/` 증거 존재 확인.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

  **QA Scenarios**:
  ```
  Scenario: Must Have/Must NOT Have 전수 검증 (happy)
    Tool: Bash (rg/actionlint/ls)
    Steps:
      1. rg -n "리뷰중|codex:lgtm|loop-state.*comment|run list.*headSha|@(main|work/)" <both repos> .github setup 2>/dev/null
      2. ls .omo/evidence/task-*.txt | wc -l  (태스크별 증거 존재)
      3. test -f .omo/evidence/task-29-loop-runs.json (live parity 증거)
    Expected Result: Must NOT Have 패턴 매치 0, 모든 태스크 증거 파일 존재, parity 증거 존재
    Failure Indicators: 금지 패턴 1건↑, 증거 누락 → REJECT(file:line 인용)
    Evidence: .omo/evidence/final-F1-compliance.md

  Scenario: Must NOT Have 위반 탐지 (negative)
    Tool: Bash (rg)
    Steps:
      1. rg -n "PAT|personal access token|GITHUB_TOKEN.*dispatches" <both repos>/.github
    Expected Result: 위반 발견 시 file:line과 함께 REJECT
    Evidence: .omo/evidence/final-F1-violations.txt
  ```

- [~] F2. **Code/Workflow Quality Review** — `unspecified-high`
  두 repo에서 `actionlint` + `pytest` 실행. 변경 워크플로우/파이썬 검토: `write-all` 금지, 최소 권한, PAT/secret 노출,
  `as any`성 우회, 빈 except, 디버그 출력, 죽은 코드/미사용 import, AI slop(과도 주석/추상화/generic 이름).
  Output: `actionlint [PASS/FAIL] | pytest [N pass/N fail] | Files [N clean/N issues] | VERDICT`

  **QA Scenarios**:
  ```
  Scenario: lint + 테스트 + 품질 (happy)
    Tool: Bash (actionlint/pytest/rg)
    Steps:
      1. actionlint <hsi>/.github/workflows/*.yml <rs>/.github/workflows/*.yml
      2. (HSI) uvx pytest tests/workflows setup/codex-review/tests -q
      3. rg -n "write-all|continue-on-error: true|::add-mask::|print\(|except:\s*$" <changed files>
    Expected Result: actionlint 0 error, pytest all pass, write-all/빈except/디버그출력 0
    Evidence: .omo/evidence/final-F2-quality.txt

  Scenario: secret 노출/권한 과다 탐지 (negative)
    Tool: Bash (rg)
    Steps:
      1. rg -n "permissions:\s*write-all|PRIVATE_KEY|relay_token=|app.*private.*key" <both repos>/.github
    Expected Result: 평문 secret/write-all 매치 0 (발견 시 FAIL)
    Evidence: .omo/evidence/final-F2-secrets.txt
  ```

- [~] F3. **Real Manual QA — dry-run + live 루프 재현** — `unspecified-high`
  clean 상태에서 모든 태스크 QA 시나리오 재실행. dry-run `repository_dispatch` smoke → 결정론적 결과 확인.
  승인된 1회 live same-repo 루프(review→…→fix-push→redispatch→LGTM 자동완주) 재현 + 종료 가시화 확인.
  cross-task 통합/엣지(stale-head, max-iter, fork, closed-PR) 확인. 증거 `.omo/evidence/final-qa/`.
  Output: `Scenarios [N/N] | Loop [self-drives to LGTM Y/N] | Edge [N tested] | VERDICT`

  **QA Scenarios**:
  ```
  Scenario: dry-run smoke + live 자동완주 재현 (happy)
    Tool: Bash (gh)
    Preconditions: 승인된 테스트 PR, 어댑터 default branch 병합
    Steps:
      1. gh api repos/<owner>/<repo>/dispatches --method POST -f event_type=codex-loop -F 'client_payload[stage]=review' -F 'client_payload[dry_run]=true' ...  → HTTP 204
      2. (승인 시) live kickoff → gh run list --json event,headSha,conclusion --limit 30 으로 review→design→fix→review… 체인 + 최종 terminal=lgtm 확인
      3. head_sha 갱신(fix-push) 추적
    Expected Result: dry-run 결정론적 종료, live는 LGTM까지 자동완주, 라벨/코멘트 상태 변동 0
    Evidence: .omo/evidence/final-qa/loop-chain.json

  Scenario: 엣지 종료 경로 (negative)
    Tool: Bash (gh/pytest)
    Steps:
      1. stale-head/max-iter/fork/closed-PR 각 케이스를 dry-run-safe로 트리거
      2. 각 run의 terminal_reason 확인(stale_head/max_iterations/fork_pr/pr_closed)
    Expected Result: 각 엣지가 올바른 terminal_reason로 종료, silent loop 0
    Evidence: .omo/evidence/final-qa/edge-cases.txt
  ```

- [~] F4. **Scope Fidelity Check** — `deep`
  각 태스크 "What to do" vs 실제 diff(git log/diff) 1:1 확인 — 누락 없음/스코프 크립 없음. "Must NOT do" 준수.
  cross-task 오염(다른 태스크 파일 침범), 미설명 변경, relayer/Rust 소스 무변경, prompt/schema 무단 "개선" 여부 확인.
  Output: `Tasks [N/N compliant] | Contamination [CLEAN/N] | Unaccounted [CLEAN/N] | VERDICT`

  **QA Scenarios**:
  ```
  Scenario: 스코프 1:1 대조 (happy)
    Tool: Bash (git)
    Steps:
      1. (각 PR 브랜치) git diff --stat <base>  로 변경 파일 목록 확보
      2. 각 태스크 "What to do"/"Must NOT do"와 실제 diff 1:1 대조(누락/크립 없음)
      3. git diff <base> -- src/ examples/ Cargo.toml (relayer/Rust 무변경 확인)
    Expected Result: 모든 태스크 1:1 준수, relayer/Rust diff 0
    Evidence: .omo/evidence/final-F4-scope.txt

  Scenario: 오염/무단개선 탐지 (negative)
    Tool: Bash (git)
    Steps:
      1. git -C <hsi> diff --stat <base> -- setup/codex-review/schemas setup/codex-review/prompts
    Expected Result: parity 목적 외 schema/prompt 변경 0 (있으면 근거 필요, 없으면 FLAG)
    Evidence: .omo/evidence/final-F4-contamination.txt
  ```

---

## Commit Strategy

> AGENTS.md: setup/docs/behavior/live-capable PR을 섞지 않는다. Wave ≈ PR. 에이전트는 **머지하지 않음**(maintainer 게이트).

- **PR1 (HSI)**: Wave 0 — PR #18 머지(=maintainer). 이후 SHA 기록.
- **PR2 (HSI, docs+contract)**: tasks 2–6 — `docs(codex-loop): payload v2 contract + topology + taxonomy`
- **PR3+PR4 (HSI, setup)**: tasks 7–11 — `chore(codex-loop): port codex-review CLI + overridable config`
- **PR5 (HSI, behavior)**: tasks 12–19 — `feat(codex-loop): full stage graphs (dry-run, no live continuation)`
- **PR6 (HSI, live)**: tasks 20–26 — `feat(codex-loop): trusted fix-push + dispatch continuation + caps`
- **PR7 (RS, consumer)**: tasks 27–29 — `ci(codex-loop): add SHA-pinned dry-run consumer adapters`
- **PR8 (RS, cleanup)**: tasks 30–32 — `chore(codex): remove label workflows + setup/codex-review`
- Pre-commit(공통): `actionlint` + 관련 `pytest` + `git diff --check`.

## Success Criteria

### Verification Commands
```bash
# 두 repo 공통
actionlint .github/workflows/*.yml                      # Expected: 0 error
# HSI 이식 경로
uvx pytest tests/workflows -q                           # Expected: all pass
uvx pytest setup/codex-review/tests -q                  # Expected: all pass (이식 후)
# dry-run dispatch smoke (HSI default branch + 테스트 PR)
gh api repos/<owner>/<repo>/dispatches --method POST \
  --field event_type=codex-loop --field client_payload[stage]=review --field client_payload[dry_run]=true ...
gh run list --workflow codex-loop-dispatch.yml --json status,conclusion --limit 5   # Expected: success, dry-run
# RS cleanup 검증
rg -n "리뷰중|리뷰완료|설계완료|수정중|codex:lgtm|codex:needs-issue" .github | wc -l    # Expected: 0
test ! -d setup/codex-review && echo "removed"          # Expected: removed
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent (라벨/코멘트 상태, PAT, fork fix-push, head_sha 탐색, 코멘트 loop 메모리)
- [ ] dry-run smoke + 1회 live 루프 parity 증거 존재
- [ ] 모든 contract/parity/negative pytest 통과, actionlint 0 error
