// Lado TS da matriz cases-parser.toml: parity/ts-check.ts com o erro do parser normalizado por
// parity/normalizar-parser.sh, como o lado Rust (parity/rust-parser.sh).
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";

const here = dirname(import.meta.path);
const r = spawnSync(join(here, "normalizar-parser.sh"), [process.execPath, join(here, "ts-check.ts"), ...process.argv.slice(2)], {
  stdio: ["inherit", "pipe", "pipe"],
});
process.stdout.write(r.stdout);
process.stderr.write(r.stderr);
process.exit(r.status ?? 1);
