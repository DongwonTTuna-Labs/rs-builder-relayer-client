# setup/codex-review

이 디렉터리는 Codex PR review v3 workflow의 실제 구현 단위가 들어있는 helper package다.

현재 상태는 실행 가능한 implementation이다. 공통 기반, GitHub helper, security policy, stage00-stage08 CLI, provider-neutral model adapter, trusted push orchestration, workflow guardrail test가 포함되어 있다. 모델 job은 `CODEX_REVIEW_MODEL_COMMAND`를 통해 repository별 runner를 연결할 수 있고, runner가 없을 때는 검증 가능한 안전 fallback artifact를 생성해 write/push side effect를 막는다.

## 구현/검증 범위

1. `config.py`, `env.py`, `paths.py`, `artifacts.py`, `schema.py` 공통 기반
2. `github/client.py`, `github/review_threads.py`, `github/issues.py` 등 read/write helper
3. `security/provenance.py`, `security/redaction.py`, `security/patch_policy.py` 보안 정책
4. stage00 resolve gate fixture/GitHub read/dry-run/actual apply path
5. stage01/02 review + techlead artifact validation/publication path
6. stage03/04 design + design chief routing/publication path
7. stage05/06/07 autofix patch dispatch/merge/push guard path
8. stage08 reentry record/validation path
9. workflow shape test로 `.github/workflows/codex-review-orchestrator.yml`을 강제

## 모델 runner 연결

workflow repository variable 또는 job env에 아래 값을 설정하면 각 model stage가 같은 command를 호출한다.

```bash
CODEX_REVIEW_MODEL_COMMAND='your-model-runner --prompt {prompt} --out {output} --schema {schema}'
```

command에는 다음 환경변수도 전달된다.

```text
CODEX_REVIEW_STAGE
CODEX_REVIEW_PROMPT_PATH
CODEX_REVIEW_OUTPUT_PATH
CODEX_REVIEW_EXPECTED_SCHEMA
```

stage별 override가 필요하면 `CODEX_REVIEW_STAGE00_MODEL_COMMAND`, `CODEX_REVIEW_STAGE01_CORRECTNESS_MODEL_COMMAND`, `CODEX_REVIEW_STAGE06_MERGE_MODEL_COMMAND`처럼 stage 이름을 붙인 환경변수를 사용할 수 있다.

## 가장 중요한 원칙

- PR head의 `setup/` 코드는 절대 실행하지 않는다.
- trusted script는 base SHA checkout에서만 실행한다.
- model output은 항상 검증 후 trusted job에서만 side effect를 적용한다.
