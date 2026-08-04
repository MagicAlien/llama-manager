// T-000 bootstrap lint.
//
// The real lint tooling (ESLint + TypeScript) arrives with T-001. Until the
// source tree exists, there is nothing for a linter to read; this script
// asserts exactly that precondition and exits zero. T-001 replaces it
// outright. It carries no dependency docs/DEV-SETUP.md does not require.
"use strict";

const fs = require("node:fs");
const path = require("node:path");

const srcDir = path.join(__dirname, "..", "src");

if (!fs.existsSync(srcDir)) {
  // No source tree yet — nothing to lint. This is the T-000 state.
  console.log("lint: no src/ directory yet; nothing to lint (T-001 adds it)");
  process.exit(0);
}

const entries = fs.readdirSync(srcDir);
if (entries.length > 0) {
  console.error(
    `lint: src/ contains ${entries.length} file(s) but the bootstrap linter cannot read them. ` +
      "Install the real lint tooling (T-001) before adding source files."
  );
  process.exit(1);
}

console.log("lint: src/ exists and is empty; nothing to lint");
process.exit(0);
