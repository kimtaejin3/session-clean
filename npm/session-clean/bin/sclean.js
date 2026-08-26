#!/usr/bin/env node
"use strict";

// 이 파일은 실행 파일이 아니라 **런처**다.
// 실제 프로그램은 Rust로 컴파일된 네이티브 바이너리이고, 플랫폼마다 다른
// optionalDependency 패키지에 들어 있다. npm이 os/cpu가 맞는 것 하나만
// 설치하므로, 여기서는 그것을 찾아 그대로 넘겨주기만 한다.

const { spawnSync } = require("node:child_process");

// 스코프(@session-clean/...)를 쓰면 같은 이름의 npm 조직을 먼저 만들어야 한다.
// 그 단계를 없애려고 스코프 없는 이름을 쓴다.
const PACKAGES = {
  "darwin-arm64": "session-clean-darwin-arm64",
  "darwin-x64": "session-clean-darwin-x64",
  "linux-x64": "session-clean-linux-x64",
  "linux-arm64": "session-clean-linux-arm64",
};

const key = `${process.platform}-${process.arch}`;
const pkg = PACKAGES[key];

if (!pkg) {
  console.error(
    `sclean: ${key} 는 아직 지원하지 않습니다.\n` +
      `지원 플랫폼: ${Object.keys(PACKAGES).join(", ")}\n` +
      `소스에서 직접 빌드할 수 있습니다: https://github.com/kimtaejin3/session-clean`,
  );
  process.exit(1);
}

let binary;
try {
  binary = require.resolve(`${pkg}/bin/sclean`);
} catch {
  // optionalDependency 설치가 건너뛰어진 경우(--no-optional, 플랫폼 불일치 등).
  console.error(
    `sclean: ${key} 용 바이너리를 찾지 못했습니다.\n` +
      `설치가 덜 된 것 같습니다. 다시 시도해 보세요:\n` +
      `  npm install ${pkg}\n` +
      `또는 --no-optional 없이 재설치하세요.`,
  );
  process.exit(1);
}

// TUI 이므로 stdio 를 그대로 물려줘야 한다. 파이프로 감싸면 터미널이
// 아니라고 판단해 프로그램이 바로 종료된다.
const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  console.error(`sclean: 실행에 실패했습니다: ${result.error.message}`);
  process.exit(1);
}
// 시그널로 죽었으면 같은 시그널로 죽은 것처럼 종료 코드를 맞춘다.
process.exit(result.status === null ? 1 : result.status);
