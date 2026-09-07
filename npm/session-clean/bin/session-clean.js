#!/usr/bin/env node
"use strict";

// This file is a launcher, not the program itself.
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
    `session-clean: ${key} is not supported yet.\n` +
      `Supported: ${Object.keys(PACKAGES).join(", ")}\n` +
      `You can build from source: https://github.com/kimtaejin3/session-clean`,
  );
  process.exit(1);
}

let binary;
try {
  binary = require.resolve(`${pkg}/bin/session-clean`);
} catch {
  // optionalDependency 설치가 건너뛰어진 경우(--no-optional, 플랫폼 불일치 등).
  console.error(
    `session-clean: no binary found for ${key}.\n` +
      `The install looks incomplete. Try:\n` +
      `  npm install ${pkg}\n` +
      `or reinstall without --no-optional.`,
  );
  process.exit(1);
}

// TUI 이므로 stdio 를 그대로 물려줘야 한다. 파이프로 감싸면 터미널이
// 아니라고 판단해 프로그램이 바로 종료된다.
const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  console.error(`session-clean: failed to run: ${result.error.message}`);
  process.exit(1);
}
// 시그널로 죽었으면 같은 시그널로 죽은 것처럼 종료 코드를 맞춘다.
process.exit(result.status === null ? 1 : result.status);
