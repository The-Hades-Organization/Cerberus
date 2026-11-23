// Estados de plugin da matriz do Limiar, montados pelo `hades plugins` da skill fixada (molde:
// loop/test/ts/cerberus-limiar-legado.test.ts do Hades-Skill). Cada estado vira <saida>/<nome>.tar.gz com `packs/` e
// `data/` do HOME; o caminho da montagem vira @HOME@ nos arquivos de texto, e parity/setup/limiar-lib.sh desfaz no
// HOME de cada lado. Git sem config do usuário, autor e data fixos: os commits saem com os shas que a matriz cita.
//   bun limiar-estado.ts <skill> <saida>
import { spawnSync } from "node:child_process";
import {
  appendFileSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, renameSync, rmSync,
  statSync, writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const [skillArg = "", outArg = ""] = process.argv.slice(2);
if (!skillArg || !outArg) {
  console.error("uso: bun limiar-estado.ts <skill> <saida>");
  process.exit(2);
}
const SKILL = resolve(skillArg), OUT = resolve(outArg);
const T = mkdtempSync(join(tmpdir(), "limiar-estado-"));
const H = join(T, "h"), RM = join(T, "remotes"), SRC = join(T, "src"), GC = join(T, "gitconfig");
const PR = join(H, "packs"), Dp = join(H, "data");
const BASE = "https://plugins.example.invalid";
/** Id fixo da prévia: o sorteado vira este, para o `pack_dir` do contexto apontar para ela. */
const ID = "0a1b2c3d4e5f6a7b";

const GIT_ENV = {
  GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@t", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@t",
  GIT_AUTHOR_DATE: "2026-01-01T00:00:00Z", GIT_COMMITTER_DATE: "2026-01-01T00:00:00Z",
};
const env = {
  PATH: process.env.PATH ?? "/usr/bin:/bin", HOME: H, TZ: "UTC", LANG: "C.UTF-8", NO_COLOR: "1",
  GIT_CONFIG_GLOBAL: GC, GIT_CONFIG_NOSYSTEM: "1", HADES_PACKS_ROOT: PR, HADES_DATA: Dp, HADES_WT: join(H, "wt"),
  HADES_SKILL_DIR: SKILL, HADES_PLUGIN_GIT_TIMEOUT: "30", ...GIT_ENV,
};

function run(argv: string[], cwd = T): string {
  const r = spawnSync(argv[0] ?? "", argv.slice(1), { cwd, env, encoding: "utf8" });
  if (r.status !== 0) {
    console.error(`falhou (${r.status}): ${argv.join(" ")}\n${r.stdout}${r.stderr}`);
    process.exit(1);
  }
  return r.stdout;
}
const git = (dir: string, ...a: string[]): string => run(["git", "-C", dir, ...a]).trim();
const sg = (...a: string[]): string => git(SRC, ...a);
const PL = (...a: string[]): Record<string, unknown> =>
  JSON.parse(run([process.execPath, join(SKILL, "src/cli.ts"), "plugins", ...a, "--json"]));

/** Arquivos de texto sob `dir` (fora dos objetos do git) que citam o caminho da montagem. */
function* textFiles(dir: string): Generator<string> {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) {
      if (name !== "objects") yield* textFiles(p);
    } else if (st.isFile()) {
      yield p;
    }
  }
}

/** Grava o estado atual de `packs/` e `data/` em <OUT>/<nome>.tar.gz, com o HOME da montagem trocado por @HOME@. */
function snapshot(name: string, extra: (stage: string) => void = () => {}): void {
  const stage = join(T, `snap-${name}`);
  rmSync(stage, { recursive: true, force: true });
  mkdirSync(stage);
  run(["cp", "-a", PR, Dp, stage]);
  extra(stage);
  for (const f of textFiles(stage)) {
    const raw = readFileSync(f);
    const text = raw.toString("utf8");
    if (!Buffer.from(text, "utf8").equals(raw) || !text.includes(H)) continue;
    const st = statSync(f);
    writeFileSync(f, text.split(H).join("@HOME@"));
    spawnSync("touch", ["-d", `@${st.mtimeMs / 1000}`, f]);
  }
  run(["tar", "-czf", join(OUT, `${name}.tar.gz`), "-C", stage, "."]);
}

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });
writeFileSync(GC, `[url "${RM}/"]\n\tinsteadOf = ${BASE}/\n`);
for (const d of [PR, join(Dp, "logs"), SRC, join(H, "wt")]) mkdirSync(d, { recursive: true });

// pacote de teste publicado num remote local
run(["git", "init", "-q", "--bare", "-b", "main", `${RM}/lim.git`]);
run(["git", "init", "-q", "-b", "main", SRC]);
mkdirSync(`${SRC}/env/panel/ui`, { recursive: true });
mkdirSync(`${SRC}/env/checks`, { recursive: true });
const prof = JSON.parse(readFileSync(`${SKILL}/loop/test/fixtures/packs/example-monorepo/profile.json`, "utf8"));
prof.name = "lim"; prof.identity.repo = "acme/lim"; prof.env = { pack: "env" }; prof.core = ">=5";
writeFileSync(`${SRC}/profile.json`, JSON.stringify(prof, null, 2) + "\n");
writeFileSync(`${SRC}/env/post-install.sh`, '#!/usr/bin/env bash\ntouch "$HADES_DATA/lim.ran"\n');
writeFileSync(`${SRC}/env/lim-svc.sh`, "#!/usr/bin/env bash\necho svc\n");
writeFileSync(`${SRC}/env/config.tsv`, "LIM_LANES\t3\tint:1:9\tProjeto LIM\tFaixas\tnext\tChave do plugin.\n");
writeFileSync(`${SRC}/env/panel/api.ts`, "export function eco() { return { ok: true }; }\n");
writeFileSync(`${SRC}/env/panel/ui/lim.js`, 'window.HadesPanel.registerPage("lim", () => null);\n');
writeFileSync(`${SRC}/env/plugin.json`, JSON.stringify({ name: "lim", panel: { pages: [{ id: "lim", label: "LIM", script: "lim.js" }], routes: [{ path: "eco", fn: "eco" }], actions: ["lim-run"] } }) + "\n");
sg("add", "-A"); sg("commit", "-qm", "v1"); sg("push", "-q", `${RM}/lim.git`, "main");

// ---- add: a prévia, com o id fixo
const added = PL("add", `${BASE}/lim.git`);
const c1 = String(added.commit);
const staging = join(Dp, "plugins-staging");
renameSync(join(staging, String(added.id)), join(staging, ID));
const prev = join(staging, ID, "preview.json");
writeFileSync(prev, readFileSync(prev, "utf8").split(String(added.id)).join(ID));
const shas: Record<string, string> = { c1 };
const pv = (): Record<string, any> => JSON.parse(readFileSync(prev, "utf8"));
snapshot("add", (stage) => {
  // variantes da prévia (as do golden), copiadas por cima pelo setup de cada caso
  const v = join(stage, "var");
  mkdirSync(v);
  const x1 = pv(); x1.runs.find((q: { path: string }) => q.path === "env/post-install.sh").sha256 = "00";
  writeFileSync(join(v, "x1.json"), JSON.stringify(x1, null, 2));
  const x2 = pv(); x2.runs = x2.runs.filter((q: { path: string }) => q.path !== "env/panel/api.ts");
  writeFileSync(join(v, "x2.json"), JSON.stringify(x2, null, 2));
  const x3 = pv(); x3.other = []; x3.adds = { pages: [{ id: "outra" }], routes: ["GET outra"] };
  writeFileSync(join(v, "x3.json"), JSON.stringify(x3, null, 2));
  const x4 = pv(); x4.runs = [{ path: "env/post-install.sh" }];
  writeFileSync(join(v, "x4.json"), JSON.stringify(x4, null, 2));
});
PL("add", "--apply", ID, "--commit", c1);

// ---- enable: o pacote instalado; e um commit local que ninguém instalou
snapshot("enable");
git(join(PR, "lim"), "commit", "-q", "--allow-empty", "-m", "local");
shas.local = git(join(PR, "lim"), "rev-parse", "HEAD");
snapshot("enable-local");
git(join(PR, "lim"), "reset", "-q", "--hard", c1);

// ---- update: c2 muda o post-install; as outras saem do c2, uma por branch
sg("checkout", "-q", "-b", "main2");
appendFileSync(`${SRC}/env/post-install.sh`, "echo v2\n");
sg("add", "-A"); sg("commit", "-qm", "v2"); sg("push", "-q", `${RM}/lim.git`, "main2:main");
sg("checkout", "-q", "main"); sg("reset", "-q", "--hard", sg("rev-parse", "main2"));
const c2 = sg("rev-parse", "HEAD");
shas.c2 = c2;
PL("update", "lim");
const mkBranch = (b: string): void => {
  sg("add", "-A"); sg("commit", "-qm", b); sg("branch", "-f", b, "HEAD"); sg("push", "-q", `${RM}/lim.git`, b);
  sg("reset", "-q", "--hard", c2); sg("clean", "-qfd");
};
writeFileSync(`${SRC}/env/extra.sh`, "#!/usr/bin/env bash\n"); mkBranch("b-novo");
appendFileSync(`${SRC}/env/lim-svc.sh`, "# v2\n"); mkBranch("b-mod");
writeFileSync(`${SRC}/env/config.sh`, "echo ok\n"); mkBranch("b-nucleo");
appendFileSync(`${SRC}/env/post-install.sh`, "cp x ~/.claude/skills/hades/loop/hooks/hades-guard.py\n"); mkBranch("b-guard");
appendFileSync(`${SRC}/env/config.tsv`, "HADES_CERBERUS_PORTA\toff\tenum:off,barra\tG\tL\tnext\tD\n"); mkBranch("b-modo");
appendFileSync(`${SRC}/env/post-install.sh`, "jq . ~/.claude/settings.json\n"); mkBranch("b-settings");
appendFileSync(`${SRC}/env/post-install.sh`, 'jq ".hooks = {}" ~/.claude/settings.json\n'); mkBranch("b-settings-hook");
mkdirSync(`${SRC}/loop/hooks`, { recursive: true }); writeFileSync(`${SRC}/loop/hooks/guard-rules.json`, "{}\n"); mkBranch("b-loop");
writeFileSync(`${SRC}/env/cerberus-x.sh`, "echo oi\n"); mkBranch("b-cerberus");
writeFileSync(`${SRC}/env/README.txt`, "ok\n"); mkBranch("b-neutro");
writeFileSync(`${SRC}/env/plugin.json`, JSON.stringify({ name: "lim", panel: { pages: [{ id: "outra", label: "O" }], routes: [{ path: "x", method: "post" }], actions: ["a", 3] } }) + "\n"); mkBranch("b-manifesto");
appendFileSync(`${SRC}/env/post-install.sh`, "tee ~/.claude/skills/hades/x\nrm -rf $HADES_SKILL_DIR/loop\n# cp skills/hades\n"); mkBranch("b-escreve");
writeFileSync(`${SRC}/env/plugin.json`, JSON.stringify({ name: "lim", panel: { pages: [{ id: "cerberus" }] } }) + "\n"); mkBranch("b-pagina");
writeFileSync(`${SRC}/profile.json`, "{nao"); mkBranch("b-perfil");
prof.env = { pack: "../fora" }; writeFileSync(`${SRC}/profile.json`, JSON.stringify(prof)); mkBranch("b-fora");
run(["git", "-C", join(PR, "lim"), "fetch", "-q", "origin"]);
for (const b of ["b-neutro", "b-novo", "b-mod", "b-nucleo", "b-guard", "b-modo", "b-settings", "b-settings-hook", "b-loop",
  "b-cerberus", "b-manifesto", "b-escreve", "b-pagina", "b-perfil", "b-fora"]) {
  shas[b] = git(join(PR, "lim"), "rev-parse", `origin/${b}`);
}
snapshot("update");

writeFileSync(join(OUT, "shas.json"), JSON.stringify(shas, null, 2) + "\n");
rmSync(T, { recursive: true, force: true });
console.log(OUT);
