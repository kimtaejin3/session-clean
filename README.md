# sclean

[![CI](https://github.com/kimtaejin3/session-clean/actions/workflows/ci.yml/badge.svg)](https://github.com/kimtaejin3/session-clean/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/session-clean)](https://www.npmjs.com/package/session-clean)
[![license](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

여러 코딩 에이전트에 쌓인 세션을 한 화면에서 확인하고, 명확한 기준에 따라 추천받아 한 번에 안전하게 정리하는 로컬 터미널 UI입니다.

```text
┌ Projects 2/5 ──────────────┐┌ Codex · shop-api — 세션 2 ─────────────────── Trash: 0 ┐
│  ── Claude Code            ││  [x] ★ 92일 전  로그인 리다이렉…  마지막 활동 후 92일…  │
│  blog                      ││  [ ] ★ 45일 전  빌드 오류 질문    사용자 메시지 1개…    │
│  shop-api        추천 2    ││                                                        │
│▶ ── Codex                  ││                                                        │
│  shop-api        추천 2    ││                                                        │
│  ── Gemini CLI (미검증)    ││                                                        │
│  a1b2c3d4        확인불가  ││                                                        │
│  ── Continue               ││                                                        │
│  shop-api        추천 1    ││                                                        │
└────────────────────────────┘└────────────────────────────────────────────────────────┘
 세션 9개 · 추천 5개 · 선택 1개 (1 KB)
 ↑↓ 세션  ← 프로젝트로  Space 선택  A 추천전체  D 정리  T 휴지통  F 기준  ? 도움말  Q 종료
```

기호만으로 상태를 구분할 수 있습니다 — `[x]` 선택 · `[ ]` 미선택 · `[-]` 정리 불가 · `★` 추천 · `!` 분석 불가 · `▶` 실행 중. 색은 거들 뿐입니다.

## 설치

```sh
npx session-clean
```

한 번만 써볼 거면 위 명령으로 충분합니다. 계속 쓰실 거면 전역 설치하세요.

```sh
npm install -g session-clean
sclean
```

패키지 이름은 `session-clean`이고 **실행 명령은 `sclean`** 입니다.

소스에서 직접 빌드하려면 Rust 1.85 이상(edition 2024)이 필요합니다.

```sh
cargo install --path .
```

### 지원 에이전트

세션 하나가 파일 하나인 에이전트만 지원합니다. sclean 의 안전 모델이 "파일을 휴지통으로 옮겼다가 되돌리는 것"이라, 세션을 SQLite 행으로 넣는 도구(Cursor, OpenCode, Goose 등)에는 그대로 적용되지 않습니다.

| 에이전트 | 저장 위치 | 형식 확인 |
|---|---|---|
| Claude Code | `~/.claude/projects/<cwd>/<uuid>.jsonl` | 실제 데이터로 확인 |
| Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` | 실제 데이터로 확인 |
| Continue | `~/.continue/sessions/<uuid>.json` | 실제 데이터로 확인 |
| Gemini CLI | `~/.gemini/tmp/<hash>/chats/*.json` | 문서 기준 |
| Copilot CLI | `~/.copilot/session-state/` | 문서 기준 |

`문서 기준`인 두 에이전트는 화면에 `(미검증)`으로 표시합니다. 실제 형식이 다르면 `분석 불가`가 되어 **정리가 차단**되므로, 잘못 지우는 대신 아무것도 하지 않습니다.

설치되지 않은 에이전트는 조용히 건너뜁니다. 각 에이전트의 정리 대상은 자기 데이터 루트 안에 있는지 검증하므로, 한 에이전트를 정리해도 다른 에이전트의 파일은 건드리지 않습니다.

### 지원 플랫폼

| 플랫폼 | 상태 |
|---|---|
| macOS arm64 / x64 | 지원 (1차 검증 환경은 Apple Silicon) |
| Linux x64 / arm64 | 지원 |
| Windows | 네이티브 미지원 — **WSL에서 사용하세요** |

Windows에서는 유닉스 전용 파일시스템 API(심볼릭 링크, 프로세스 시그널)를 써서 컴파일되지 않습니다.
WSL 안에서 Claude Code를 쓰신다면 `~/.claude`도 WSL에 있으므로 Linux 빌드가 그대로 동작합니다.

설정과 휴지통은 macOS에서 `~/Library/Application Support/sclean`, Linux에서 `~/.local/share/sclean`에 저장됩니다.

## 실행

```sh
sclean
```

별도 옵션 없이 실행되며 세션 선택, 기준 변경, 휴지통 이동, 완전 삭제, 복원은 모두 TUI 안에서 처리합니다.

## 조작

왼쪽에는 프로젝트 목록, 오른쪽에는 **선택한 프로젝트의 세션**이 표시됩니다. `→`로 세션 목록으로 이동하고 `←`로 프로젝트 목록으로 돌아옵니다.
터미널 폭이 좁으면 현재 선택한 패널만 전체 폭으로 표시합니다.

| 키 | 동작 |
|---|---|
| `↑` `↓` | 현재 패널에서 항목 이동 |
| `→` | 선택한 프로젝트의 세션 목록으로 이동 |
| `←` | 프로젝트 목록으로 돌아가기 |
| `Space` | 선택·해제 (프로젝트 목록에서는 그 프로젝트 전체) |
| `A` | 추천 항목 전체 선택·해제 (모든 프로젝트) |
| `D` | 선택 항목 정리 |
| `T` | 휴지통 화면 |
| `F` | 추천 기준 화면 |
| `?` | 도움말 |
| `Q` | 종료 |

정리 확인 화면에서는 `←` `→`로 `휴지통 이동` 또는 `완전 삭제`를 고릅니다. 완전 삭제는 `DELETE`를 직접 입력해야 실행됩니다.

휴지통 화면에서는 `R` 복원, `X` 영구 삭제, `Enter` 상세 보기입니다.

## 추천 규칙

아래 규칙 중 하나라도 충족하면 `Recommended`로 표시하고, 추천 이유를 한 줄로 보여줍니다. 각 규칙은 `F` 화면에서 켜거나 끌 수 있습니다.

| 규칙 | 조건 | 기본값 |
|---|---|---|
| R1 오래된 세션 | 마지막 활동이 기준일보다 오래됨 | 켜짐 / 30일 |
| R2 존재하지 않는 프로젝트 | 세션의 `cwd`가 현재 파일시스템에 없음 | 켜짐 |
| R3 짧은 세션 | 사용자 메시지 1개 이하 + 도구 실행 없음 | 켜짐 |
| R4 종료된 하위 에이전트 | 하위 에이전트 세션이고 최근 활동이 없음 | 켜짐 |
| R5 고아 데이터 | 대화 기록 없이 연결 데이터만 남음 | 켜짐 |

다음 중 하나라도 해당하면 **추천하지 않거나 정리를 차단**합니다.

- 세션 형식을 분석할 수 없음 (차단)
- 연결 데이터의 소유 세션을 확정할 수 없음 (차단)
- 현재 실행 중인 세션 (차단)
- 스캔 이후 파일의 크기나 수정 시각이 바뀜 (차단)
- 프로젝트 경로를 신뢰할 수 있게 확인하지 못함 (추천 보류)
- 5분 안에 활동한 세션 (추천 보류)

## 다루는 데이터

`sclean`은 아래 경로 중 실제로 존재하는 곳만 읽습니다. 경로가 없어도 오류로 처리하지 않습니다.

**Claude Code**

```text
~/.claude/projects/<프로젝트>/<세션>.jsonl   대화 기록
~/.claude/projects/<프로젝트>/<세션>/        하위 에이전트 기록
~/.claude/tasks/  teams/                     세션 ID 앞 8자리로 연결 (하나만 일치할 때만 해당 세션의 데이터로 판단)
~/.claude/session-env/  file-history/  todos/  debug/
~/.claude/sessions/                          실행 중 세션 잠금 (읽기만)
~/.claude/history.jsonl                      공유 기록 (소유가 확정되는 줄만 수정)
```

**그 밖의 에이전트**

```text
~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl   세션 (첫 줄에 cwd 가 있음)
~/.codex/archived_sessions/                    보관된 세션
~/.continue/sessions/<uuid>.json               세션 (title · workspaceDirectory 포함)
~/.gemini/tmp/<project_hash>/chats/*.json      세션 (프로젝트는 해시로만 표현)
~/.copilot/session-state/                      세션
```

`sclean` 자체 데이터는 아래 경로에만 저장합니다.

```text
~/Library/Application Support/sclean/   (Linux: ~/.local/share/sclean/)
├── config.json      추천 기준
├── sclean.log       로컬 로그 (프롬프트 본문은 기록하지 않습니다)
└── trash/<작업ID>/  manifest.json + 옮겨둔 파일
```

프로젝트 소스 파일은 **존재 확인 외에 읽지도 쓰지도 않습니다.**

## 안전 장치

- 실행 직전에 모든 대상 파일을 다시 확인하고, 스캔 이후 바뀐 세션은 제외합니다.
- 모든 대상 경로가 `~/.claude` 안에 있는지 확인합니다. 하나라도 범위를 벗어나면 작업 전체를 실행하지 않습니다. 심볼릭 링크는 따라가지 않습니다.
- 파일을 옮기기 **전에** `manifest.json`을 먼저 기록합니다. 도중에 강제 종료되어도 다음 실행에서 복구를 제안합니다.
- `history.jsonl`처럼 공유되는 파일은 백업 후 임시 파일에 쓰고 원자적으로 교체합니다.
- 복원할 때 원래 경로에 파일이 있으면 덮어쓰지 않고 충돌로 표시합니다.
- 삭제와 복원은 반복 실행해도 안전합니다.

## 개발

```sh
cargo test                       # 182개 테스트
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --release --test perf_test -- --nocapture   # 2,000 세션 스캔 시간
```

테스트는 실제 `~/.claude`를 건드리지 않고 임시 디렉터리에 만든 테스트 데이터(fixture)만 사용합니다.
직접 확인해야 할 때는 데이터 위치를 바꿔 실행할 수 있습니다.

```sh
SCLEAN_CLAUDE_DIR=/tmp/fake-claude SCLEAN_DATA_DIR=/tmp/fake-sclean sclean
```

화면 배치를 눈으로 확인하려면 렌더 결과를 그대로 찍어볼 수 있습니다.

```sh
cargo test --test render_test visual_dump -- --ignored --nocapture
```

### 배포

npm에는 플랫폼별로 미리 빌드한 네이티브 바이너리를 올리고, 얇은 JS 런처가 `os`/`cpu`에 맞는 것을 찾아 실행합니다.

```text
session-clean                      ← 사용자가 설치하는 것 (bin/sclean.js 런처)
├── session-clean-darwin-arm64     ← optionalDependencies. npm이 맞는 것 하나만 설치
├── session-clean-darwin-x64
├── session-clean-linux-x64
└── session-clean-linux-arm64
```

`darwin-x64`는 Intel 러너 대기열이 길어 arm64 러너에서 크로스 컴파일합니다. 그 타깃은 실행할 수 없으므로 빌드만 검증합니다.

태그를 밀면 GitHub Actions가 네 플랫폼을 빌드·테스트하고 npm과 GitHub 릴리스에 올립니다.

```sh
git tag v0.1.0 && git push origin v0.1.0
```

CI 토큰이 npm의 2FA 정책에 막히면, 이미 빌드된 산출물을 받아 로컬에서 올릴 수 있습니다.

```sh
npm login                                  # OTP 직접 입력
./scripts/publish-local.sh <run-id>        # gh run list --workflow=release.yml
```

## 아직 하지 않는 것 (v0.1)

- 프로젝트·세션 검색(`/`)
- 세션 실행·재개와 실시간 모니터링
- AI 기반 분류와 백그라운드 자동 정리
- 클라우드 동기화와 네이티브 GUI
- Codex·OpenCode 지원
- 네이티브 Windows 지원 (WSL로 대체)
- Homebrew 배포
- 공유 paste cache·전역 plan·telemetry 정리

## 라이선스

MIT — [LICENSE](LICENSE)
