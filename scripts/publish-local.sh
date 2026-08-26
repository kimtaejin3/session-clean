#!/usr/bin/env bash
# CI 가 이미 빌드해 둔 바이너리를 내려받아 이 컴퓨터에서 npm 에 올린다.
#
# CI 토큰이 2FA 정책에 막힐 때 쓰는 우회로다. 로컬에서는 `npm login` 으로
# OTP 를 직접 입력하므로 토큰 설정과 무관하게 배포할 수 있다.
#
#   npm login                        # 한 번만
#   ./scripts/publish-local.sh <run-id>
#
# run-id 는 빌드가 성공한 Release 워크플로 실행 번호다.
#   gh run list --workflow=release.yml

set -euo pipefail

RUN_ID="${1:-}"
if [ -z "$RUN_ID" ]; then
  echo "사용법: $0 <run-id>" >&2
  echo "  gh run list --workflow=release.yml 로 확인하세요." >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(node -p "require('$ROOT/npm/session-clean/package.json').version")"
WORK="$ROOT/.publish-artifacts"

echo "== 버전 $VERSION, 실행 $RUN_ID =="

if ! npm whoami >/dev/null 2>&1; then
  echo "npm 에 로그인되어 있지 않습니다. 먼저 'npm login' 을 실행하세요." >&2
  exit 1
fi

rm -rf "$WORK"
gh run download "$RUN_ID" --dir "$WORK"

for key in darwin-arm64 darwin-x64 linux-x64 linux-arm64; do
  binary="$WORK/sclean-$key/sclean"
  if [ ! -f "$binary" ]; then
    echo "빌드 산출물이 없습니다: $binary" >&2
    exit 1
  fi
  node "$ROOT/npm/platforms/build.mjs" "$key" "$binary" "$VERSION"
done

# 플랫폼 패키지를 먼저 올려야 메인 패키지의 optionalDependencies 가 해석된다.
for key in darwin-arm64 darwin-x64 linux-x64 linux-arm64; do
  echo "== publish session-clean-$key =="
  npm publish "$ROOT/npm/platforms/$key" --access public
done

cp "$ROOT/README.md" "$ROOT/LICENSE" "$ROOT/npm/session-clean/"
echo "== publish session-clean =="
npm publish "$ROOT/npm/session-clean" --access public

rm -rf "$WORK"
echo
echo "완료. 확인:  npx session-clean@$VERSION --version"
