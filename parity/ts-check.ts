// Lado TS dos casos `check` da matriz: `bun ts-check.ts <skill> check <alvo>` roda `hades cerberus-check <alvo>` da
// skill montada, com CERBERUS_SKILL = skill como o executor do Cerberus passa (o binário cai no HADES_SKILL_DIR).
const [skill = "", sub = "", ...rest] = process.argv.slice(2);
process.env.CERBERUS_SKILL ??= skill;
const cli = `${skill}/src/cli.ts`;
process.argv = [process.argv[0] ?? "bun", cli, sub === "check" ? "cerberus-check" : sub, ...rest];
await import(cli);
