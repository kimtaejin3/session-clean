// 플랫폼 패키지를 만들어 낸다. CI 가 빌드한 바이너리를 넣고 npm publish 한다.
//
//   node npm/platforms/build.mjs <target-key> <binary-path> <version>
//   예: node npm/platforms/build.mjs darwin-arm64 target/aarch64-apple-darwin/release/sclean 0.1.0
//
// 결과: npm/platforms/<target-key>/{package.json,bin/sclean}

import { mkdirSync, copyFileSync, writeFileSync, chmodSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const TARGETS = {
  "darwin-arm64": { os: "darwin", cpu: "arm64" },
  "darwin-x64": { os: "darwin", cpu: "x64" },
  "linux-x64": { os: "linux", cpu: "x64" },
  "linux-arm64": { os: "linux", cpu: "arm64" },
};

const [key, binaryPath, version] = process.argv.slice(2);
const spec = TARGETS[key];
if (!spec || !binaryPath || !version) {
  console.error(
    `사용법: node build.mjs <${Object.keys(TARGETS).join("|")}> <binary> <version>`,
  );
  process.exit(1);
}

const here = dirname(fileURLToPath(import.meta.url));
const outDir = join(here, key);
mkdirSync(join(outDir, "bin"), { recursive: true });

const target = join(outDir, "bin", "sclean");
copyFileSync(binaryPath, target);
chmodSync(target, 0o755);

writeFileSync(
  join(outDir, "package.json"),
  JSON.stringify(
    {
      name: `@session-clean/${key}`,
      version,
      description: `sclean native binary for ${key}`,
      license: "MIT",
      repository: {
        type: "git",
        url: "git+https://github.com/kimtaejin3/session-clean.git",
      },
      os: [spec.os],
      cpu: [spec.cpu],
      files: ["bin"],
    },
    null,
    2,
  ) + "\n",
);

console.log(`built npm/platforms/${key}`);
