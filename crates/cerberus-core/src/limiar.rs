//! Guardas do Limiar (`src/lib/cerberus-limiar.ts`): o momento `plugin`. Tudo é lido do objeto do commit
//! (`git ls-tree`/`cat-file`), nunca do que está solto na pasta. A regra do que executa (`RUNS`), a prévia e o log de
//! auditoria são do Hades (`hades_core::plugins`): aqui só se confere. Falha ao conferir vira `barra` com a evidência
//! do que faltou, nunca `ok` calado.
//!
//! A quarta guarda, `ensaio`, roda os scripts do plugin no sandbox bwrap de [`crate::ensaio`].

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use fancy_regex::Regex as FancyRegex;
use hades_core::config::split_lines;
use hades_core::paths::{packs_root, skill_root};
use hades_core::plugins::{ENV_SKIP, PLUGIN_FILES, RUNS, is_valid_name};
use indexmap::IndexMap;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::check::{CheckEnv, CheckVerdict, Evid, ev, fnmatch_case, js_slice};
use crate::ensaio::{Arquivo, EnsaioIn, ensaiar, escrita_declarada, rede_declarada};
use crate::modelo::{EVID_MAX, Resultado, js_space, js_trim, py_repr};

use Resultado::{Aviso, Barra, Ok as Passa};

const NOT_COPIED: [&str; 5] = [
    "post-install.sh",
    "renamed.tsv",
    "assets",
    "reference",
    "__pycache__",
];
const OPS: [&str; 3] = ["add", "update", "enable"];
const SCAN_MAX: usize = 1_000_000;
const HASH_MAX: usize = 1 << 30;
const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const GIT_BUFFER_MAX: usize = 256 * 1024 * 1024;
const SHORT_SHA: usize = 12;
const DETAIL_MAX: usize = 1000;
const SNIPPET: usize = 120;
const SNIPPET_SETTINGS: usize = 100;
const LISTED_MAX: usize = 6;
const NAMED_MAX: usize = 5;
const MINUS: &str = "\u{2212}";
const POLL: Duration = Duration::from_millis(5);

/// O uso que `checks/limiar.py` imprimia com guarda desconhecida (`src/commands/cerberus-check.ts:27`).
pub const USO: &str = "uso: limiar.py commit-fixado|executavel|invariante|ensaio";

/// `\s` do JS (sem o U+0085 do `\s` Unicode do `regex`, com o BOM).
pub(crate) const JS_S: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";
/// `\p{L}\p{N}_` (`src/lib/cerberus-limiar.ts:31`): `\w` e `\b` do Python são Unicode; a lookbehind é de 1 caractere.
const W: &str = r"\p{L}\p{N}_";

/// `TAMPER` (`src/lib/cerberus-limiar.ts:35`), casado contra a linha em minúsculas ASCII (o `i` do JS sem `u` não
/// dobra `ſ` nem o sinal de Kelvin, que o `(?i)` do `regex` dobraria).
static TAMPER: LazyLock<[(&str, regex::Regex); 2]> = LazyLock::new(|| {
    [
        (
            "mexe no hook hades-guard",
            regex::Regex::new(r"hades[-_]guard").expect("padrão fixo"),
        ),
        (
            "mexe no Cerberus (guarda, modo ou histórico)",
            regex::Regex::new(
                r"hades[-_]cerberus|loop/cerberus|_maquina/cerberus|api/cerberus|hades_cerberus_",
            )
            .expect("padrão fixo"),
        ),
    ]
});
/// `SETTINGS` (`src/lib/cerberus-limiar.ts:39`), sobre a linha em minúsculas ASCII.
static SETTINGS: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"settings(\.local)?\.json").expect("padrão fixo"));
/// `SKILL_WRITE_TARGET` (`src/lib/cerberus-limiar.ts:40`).
static SKILL_WRITE_TARGET: LazyLock<FancyRegex> = LazyLock::new(|| {
    FancyRegex::new(&format!(r"skills/hades|HADES_SKILL(_DIR)?(?![{W}])")).expect("padrão fixo")
});
/// `SKILL_WRITE_ACTION` (`src/lib/cerberus-limiar.ts:41`).
static SKILL_WRITE_ACTION: LazyLock<FancyRegex> = LazyLock::new(|| {
    FancyRegex::new(&format!(
        r#"(?<![{W}])(cp|mv|ln|rsync|tee|install|rm|chmod)(?![{W}])|sed[{JS_S}]+-i|>>?[{JS_S}]*[^{JS_S}]|open\([^)]*['"][wa]"#
    ))
    .expect("padrão fixo")
});

/// `Falha` (`src/lib/cerberus-limiar.ts:46`): não consegui conferir; vira barra com a evidência do que faltou.
#[derive(Debug)]
struct Falha(String);

impl From<io::Error> for Falha {
    fn from(e: io::Error) -> Self {
        Falha(e.to_string())
    }
}

type R<T> = Result<T, Falha>;
type Verdict = (Resultado, Vec<Evid>, String);
type Guard = fn(&LimiarCtx, &CheckEnv) -> R<Verdict>;

/// `LimiarCtx` (`src/lib/cerberus-limiar.ts:52`).
struct LimiarCtx {
    plugin: String,
    commit: String,
    op: String,
    pack_dir: Option<String>,
}

fn is_sha(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn short(sha: &str) -> String {
    js_slice(sha, SHORT_SHA)
}

/// `.sort()` do JS: por unidade UTF-16.
fn js_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn sorted<'a>(items: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    let mut out: Vec<String> = items.into_iter().cloned().collect();
    out.sort_by(|a, b| js_cmp(a, b));
    out
}

fn repr_str(text: &str) -> String {
    py_repr(&Value::String(text.to_owned()))
}

/// `orDash` (`src/lib/cerberus-limiar.ts:58`).
fn or_dash(items: &[String]) -> String {
    if items.is_empty() {
        "-".into()
    } else {
        py_repr(&Value::Array(
            items.iter().cloned().map(Value::String).collect(),
        ))
    }
}

/// `pyStr` (`src/lib/cerberus-limiar.ts:65`): `str()` de um valor lido do JSON, como o Python o mostra.
fn py_str(value: Option<&Value>) -> String {
    match value {
        None => "None".into(),
        Some(Value::String(s)) => s.clone(),
        Some(v) => py_repr(v),
    }
}

/// Truthy do JS sobre um valor JSON.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn as_dict(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value.and_then(Value::as_object)
}

/// `parseJson` (`src/lib/cerberus-limiar.ts:69`).
fn parse_json(raw: &[u8], what: &str) -> R<Value> {
    serde_json::from_str(&String::from_utf8_lossy(raw)).map_err(|e| Falha(format!("{what}: {e}")))
}

/// `dirname` do Node (POSIX).
fn dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() {
            ".".into()
        } else {
            "/".into()
        };
    }
    match trimmed.rfind('/') {
        None => ".".into(),
        Some(0) => "/".into(),
        Some(i) => trimmed[..i].to_owned(),
    }
}

fn texto(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn is_file(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file())
}

fn is_dir(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_dir())
}

// ---------------------------------------------------------------- git pelo objeto

enum Fim {
    Saiu(Option<i32>),
    Tempo,
    Buffer,
}

fn drena(
    mut pipe: impl Read + Send + 'static,
    estourou: Arc<AtomicBool>,
) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 65536];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.len() > GIT_BUFFER_MAX {
                        estourou.store(true, AtomicOrdering::SeqCst);
                        break;
                    }
                }
            }
        }
        buf
    })
}

/// `gitRaw` (`src/lib/cerberus-limiar.ts:79`): o filho recebe só o ambiente do check (o `env` do `spawnSync`), com
/// `GIT_TERMINAL_PROMPT=0`, 30 s e 256 MiB por saída. As mensagens de erro do `spawnSync` saem com o código do Node.
fn git_raw(ce: &CheckEnv, pack: &str, args: &[&str]) -> R<Vec<u8>> {
    let first = args.first().copied().unwrap_or("");
    let nao_rodou = |code: &str| {
        Falha(format!(
            "git {first} em {pack} não rodou: spawnSync git {code}"
        ))
    };
    let mut cmd = Command::new("git");
    cmd.args([
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
        "-C",
        pack,
    ])
    .args(args)
    .env_clear()
    .envs(&ce.env)
    .env("GIT_TERMINAL_PROMPT", "0")
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(nao_rodou("ENOENT")),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(nao_rodou("EACCES")),
        Err(e) => return Err(nao_rodou(&e.to_string())),
    };
    let estourou = Arc::new(AtomicBool::new(false));
    let out = drena(child.stdout.take().expect("stdout piped"), estourou.clone());
    let err = drena(child.stderr.take().expect("stderr piped"), estourou.clone());
    let inicio = Instant::now();
    let fim = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Fim::Saiu(s.code()),
            Ok(None) => {}
            Err(_) => break Fim::Saiu(None),
        }
        let parar = if inicio.elapsed() >= GIT_TIMEOUT {
            Some(Fim::Tempo)
        } else if estourou.load(AtomicOrdering::SeqCst) {
            Some(Fim::Buffer)
        } else {
            None
        };
        if let Some(f) = parar {
            let _ = child.kill();
            let _ = child.wait();
            break f;
        }
        thread::sleep(POLL);
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    let fim = if estourou.load(AtomicOrdering::SeqCst) {
        Fim::Buffer
    } else {
        fim
    };
    match fim {
        Fim::Tempo => Err(nao_rodou("ETIMEDOUT")),
        Fim::Buffer => Err(nao_rodou("ENOBUFS")),
        Fim::Saiu(Some(0)) => Ok(stdout),
        Fim::Saiu(code) => {
            let stderr = String::from_utf8_lossy(&stderr);
            let last = split_lines(js_trim(&stderr))
                .pop()
                .unwrap_or_else(|| "sem saída".into());
            let code = code.map_or_else(|| "null".to_owned(), |c| c.to_string());
            Err(Falha(format!("git {first} em {pack} saiu {code}: {last}")))
        }
    }
}

/// `gitTxt` (`src/lib/cerberus-limiar.ts:89`).
fn git_txt(ce: &CheckEnv, pack: &str, args: &[&str]) -> R<String> {
    let raw = git_raw(ce, pack, args)?;
    Ok(js_trim(&String::from_utf8_lossy(&raw)).to_owned())
}

struct Entry {
    mode: String,
    blob: String,
}

/// `Tree` (`src/lib/cerberus-limiar.ts:92`): o conteúdo de um commit do pacote, listado e lido pelo objeto, sem
/// checkout.
struct Tree<'a> {
    files: IndexMap<String, Entry>,
    ce: &'a CheckEnv,
    pack: String,
    commit: String,
}

impl<'a> Tree<'a> {
    fn new(ce: &'a CheckEnv, pack: &str, commit: &str) -> R<Self> {
        let raw = git_raw(ce, pack, &["ls-tree", "-r", "-z", commit])?;
        let mut files = IndexMap::new();
        for ent in String::from_utf8_lossy(&raw).split('\0') {
            if ent.is_empty() {
                continue;
            }
            let tab = ent.find('\t').unwrap_or(ent.len());
            let mut campos = ent[..tab].split(js_space).filter(|s| !s.is_empty());
            let (mode, kind, blob) = (campos.next(), campos.next(), campos.next());
            if kind == Some("blob") {
                let path = ent.get(tab + 1..).unwrap_or("").to_owned();
                files.insert(
                    path,
                    Entry {
                        mode: mode.unwrap_or("").to_owned(),
                        blob: blob.unwrap_or("").to_owned(),
                    },
                );
            }
        }
        Ok(Self {
            files,
            ce,
            pack: pack.to_owned(),
            commit: commit.to_owned(),
        })
    }

    fn read(&self, path: &str, limit: usize) -> R<Vec<u8>> {
        let entry = self
            .files
            .get(path)
            .ok_or_else(|| Falha(format!("'{path}'")))?;
        let mut body = self.blob_text(&entry.blob)?;
        body.truncate(limit);
        Ok(body)
    }

    fn blob_text(&self, blob: &str) -> R<Vec<u8>> {
        git_raw(self.ce, &self.pack, &["cat-file", "blob", blob])
    }

    fn profile(&self) -> R<Map<String, Value>> {
        if !self.files.contains_key("profile.json") {
            return Err(Falha(format!(
                "o commit {} não tem profile.json",
                short(&self.commit)
            )));
        }
        let prof = parse_json(
            &self.read("profile.json", SCAN_MAX)?,
            &format!("profile.json do commit {} ilegível", short(&self.commit)),
        )?;
        Ok(match prof {
            Value::Object(m) => m,
            _ => Map::new(),
        })
    }

    fn env_prefix(&self) -> R<Option<String>> {
        let prof = self.profile()?;
        let raw = as_dict(prof.get("env")).and_then(|e| e.get("pack"));
        let Some(Value::String(raw)) = raw else {
            return Ok(None);
        };
        let trimmed = raw.trim_matches('/');
        if trimmed.is_empty() {
            return Ok(None);
        }
        if trimmed.split('/').any(|p| p == "..") {
            return Err(Falha(format!(
                "env.pack fora do pacote: {}",
                repr_str(trimmed)
            )));
        }
        Ok(Some(format!("{trimmed}/")))
    }

    /// `{caminho relativo ao env/: blob}`.
    fn env_files(&self) -> R<IndexMap<String, String>> {
        let mut out = IndexMap::new();
        let Some(prefix) = self.env_prefix()? else {
            return Ok(out);
        };
        for (path, entry) in &self.files {
            if let Some(rel) = path.strip_prefix(prefix.as_str()) {
                out.insert(rel.to_owned(), entry.blob.clone());
            }
        }
        Ok(out)
    }
}

fn topo(rel: &str) -> &str {
    rel.split('/').next().unwrap_or("")
}

/// `inPreview` (`src/lib/cerberus-limiar.ts:155`).
fn in_preview(rel: &str) -> bool {
    RUNS.iter().any(|pat| fnmatch_case(rel, pat))
}

/// `isExec` (`src/lib/cerberus-limiar.ts:148`): a regra `RUNS`, os outros `.sh` do `env/` e os `.js`/`.ts` do
/// `panel/`.
fn is_exec(rel: &str) -> bool {
    if in_preview(rel) {
        return true;
    }
    let top = topo(rel);
    if ENV_SKIP.contains(&top) {
        return false;
    }
    rel.ends_with(".sh") || (top == "panel" && (rel.ends_with(".js") || rel.ends_with(".ts")))
}

/// `Manifest` (`src/lib/cerberus-limiar.ts:157`).
struct Manifest {
    files: IndexMap<String, String>,
    pages: BTreeSet<String>,
    routes: BTreeSet<String>,
    actions: BTreeSet<String>,
}

/// `manifest` (`src/lib/cerberus-limiar.ts:160`).
fn manifest(tree: &Tree) -> R<Manifest> {
    let env = tree.env_files()?;
    let files: IndexMap<String, String> = env
        .iter()
        .filter(|(rel, _)| is_exec(rel))
        .map(|(r, b)| (r.clone(), b.clone()))
        .collect();
    let mut m = Manifest {
        files,
        pages: BTreeSet::new(),
        routes: BTreeSet::new(),
        actions: BTreeSet::new(),
    };
    let Some(blob) = env.get("plugin.json") else {
        return Ok(m);
    };
    let pj = parse_json(
        &tree.blob_text(blob)?,
        &format!("plugin.json do commit {} ilegível", short(&tree.commit)),
    )?;
    let vazio = Map::new();
    let panel = as_dict(pj.get("panel")).unwrap_or(&vazio);
    let items = |key: &str| {
        panel
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    for page in items("pages") {
        if let Some(p) = page.as_object() {
            m.pages.insert(py_str(p.get("id")));
        }
    }
    for route in items("routes") {
        let Some(r) = route.as_object() else { continue };
        let method = if truthy(r.get("method")) {
            py_str(r.get("method")).to_uppercase()
        } else {
            "GET".into()
        };
        m.routes
            .insert(format!("{method} {}", py_str(r.get("path"))));
    }
    match panel.get("actions") {
        Some(Value::String(s)) => m.actions.extend(s.chars().map(String::from)),
        Some(Value::Array(list)) => m
            .actions
            .extend(list.iter().filter_map(Value::as_str).map(str::to_owned)),
        _ => {}
    }
    Ok(m)
}

/// `sha256Of` (`src/lib/cerberus-limiar.ts:184`).
fn sha256_of(tree: &Tree, rel: &str) -> R<String> {
    let prefix = tree.env_prefix()?.unwrap_or_else(|| "null".into());
    let body = tree.read(&format!("{prefix}{rel}"), HASH_MAX)?;
    Ok(Sha256::digest(&body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

// ---------------------------------------------------------------- contexto e onde está o pacote

/// `context` (`src/lib/cerberus-limiar.ts:189`).
fn context(ce: &CheckEnv) -> R<LimiarCtx> {
    let get = |k: &str| ce.ctx.get(k).cloned().unwrap_or_default();
    let missing: Vec<&str> = ["plugin", "commit", "op"]
        .into_iter()
        .filter(|k| js_trim(&get(k)).is_empty())
        .collect();
    if !missing.is_empty() {
        return Err(Falha(format!(
            "o contexto do momento plugin não trouxe: {} (--ctx plugin=… commit=… op=add|update|enable)",
            missing.join(", ")
        )));
    }
    let (plugin, commit, op) = (get("plugin"), get("commit"), get("op"));
    if !OPS.contains(&op.as_str()) {
        return Err(Falha(format!(
            "op desconhecido: {} (add, update ou enable)",
            repr_str(&op)
        )));
    }
    if !is_valid_name(&plugin) {
        return Err(Falha(format!(
            "nome de plugin inválido: {}",
            repr_str(&plugin)
        )));
    }
    Ok(LimiarCtx {
        plugin,
        commit,
        op,
        pack_dir: ce.ctx("pack_dir").map(str::to_owned),
    })
}

fn skill_de(ce: &CheckEnv) -> PathBuf {
    if ce.skill.is_empty() {
        skill_root(&ce.hades_env())
    } else {
        PathBuf::from(&ce.skill)
    }
}

/// `pluginsCtx` (`src/lib/cerberus-limiar.ts:200`): só o `data`, o que a guarda lê.
fn data_de(ce: &CheckEnv) -> PathBuf {
    match ce.var("HADES_DATA").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => skill_de(ce).join("kit"),
    }
}

fn staging_dir(ce: &CheckEnv) -> PathBuf {
    data_de(ce).join("plugins-staging")
}

fn audit_file(ce: &CheckEnv) -> PathBuf {
    data_de(ce).join("logs").join("plugins.log")
}

/// `installedPack` (`src/lib/cerberus-limiar.ts:205`).
fn installed_pack(ctx: &LimiarCtx, ce: &CheckEnv) -> R<String> {
    let root = packs_root(&ce.hades_env());
    let pack = ctx
        .pack_dir
        .clone()
        .unwrap_or_else(|| texto(&root.join(&ctx.plugin)));
    if !Path::new(&pack).join(".git").exists() {
        return Err(Falha(format!(
            "não há o pacote git {} em {}",
            ctx.plugin,
            texto(&root)
        )));
    }
    Ok(pack)
}

/// `readPreview` (`src/lib/cerberus-limiar.ts:212`).
fn read_preview(file: &Path) -> R<Map<String, Value>> {
    let raw = fs::read(file)?;
    match parse_json(&raw, &format!("{} ilegível", texto(file)))? {
        Value::Object(m) => Ok(m),
        _ => Err(Falha(format!("{} não é um objeto", texto(file)))),
    }
}

/// `previewMatches` (`src/lib/cerberus-limiar.ts:218`).
fn preview_matches(file: &Path, ctx: &LimiarCtx) -> bool {
    read_preview(file).is_ok_and(|pv| {
        pv.get("name").and_then(Value::as_str) == Some(ctx.plugin.as_str())
            && pv.get("commit").and_then(Value::as_str) == Some(ctx.commit.as_str())
    })
}

fn mtime_ms(path: &Path) -> f64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

struct Staged {
    dir: String,
    preview: Map<String, Value>,
}

/// `staged` (`src/lib/cerberus-limiar.ts:244`): o clone em prévia e o `preview.json` do plugin com este commit; a
/// mais recente vale.
fn staged(ctx: &LimiarCtx, ce: &CheckEnv) -> R<Staged> {
    let staging = staging_dir(ce);
    let mut found: Option<(String, PathBuf)> = None;
    if let Some(pack_dir) = &ctx.pack_dir {
        let file = Path::new(&dirname(pack_dir)).join("preview.json");
        if is_file(&file) {
            found = Some((pack_dir.clone(), file));
        }
    } else if is_dir(&staging) {
        let mut ids: Vec<String> = fs::read_dir(&staging)?
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        ids.sort_by(|a, b| js_cmp(a, b));
        let mut files: Vec<PathBuf> = ids
            .iter()
            .map(|id| staging.join(id).join("preview.json"))
            .filter(|f| is_file(f))
            .filter(|f| f.parent().is_some_and(is_dir) && preview_matches(f, ctx))
            .collect();
        files.sort_by(|a, b| {
            mtime_ms(b)
                .partial_cmp(&mtime_ms(a))
                .unwrap_or(Ordering::Equal)
        });
        if let Some(newest) = files.into_iter().next() {
            let name = py_str(read_preview(&newest)?.get("name"));
            let dir = newest.parent().unwrap_or(Path::new("")).join(name);
            found = Some((texto(&dir), newest));
        }
    }
    let Some((dir, file)) = found else {
        return Err(Falha(format!(
            "nenhuma prévia de {} @ {} em {} (expirou ou nunca foi mostrada)",
            ctx.plugin,
            short(&ctx.commit),
            texto(&staging)
        )));
    };
    Ok(Staged {
        dir,
        preview: read_preview(&file)?,
    })
}

/// `audited` (`src/lib/cerberus-limiar.ts:262`): linhas do log de auditoria dos plugins como dicts.
fn audited(ce: &CheckEnv) -> R<Vec<HashMap<String, String>>> {
    let file = audit_file(ce);
    let text = match fs::read(&file) {
        Ok(raw) => String::from_utf8_lossy(&raw).into_owned(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Falha(format!("não consegui ler {}: {e}", texto(&file)))),
    };
    Ok(split_lines(&text)
        .iter()
        .map(|ln| js_trim(ln))
        .filter(|ln| !ln.is_empty())
        .map(|ln| {
            ln.split(js_space)
                .filter(|s| !s.is_empty())
                .skip(1)
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect()
        })
        .collect())
}

fn primeira_linha(text: &str) -> String {
    split_lines(text).into_iter().next().unwrap_or_default()
}

fn get<'m>(a: &'m HashMap<String, String>, k: &str) -> Option<&'m str> {
    a.get(k).map(String::as_str)
}

// ---------------------------------------------------------------- guarda: commit-fixado

/// `commitFixado` (`src/lib/cerberus-limiar.ts:279`).
fn commit_fixado(ctx: &LimiarCtx, ce: &CheckEnv) -> R<Verdict> {
    let LimiarCtx {
        plugin, commit, op, ..
    } = ctx;
    let mut evid = Vec::new();
    if !is_sha(commit) {
        return Ok((
            Barra,
            vec![ev(
                format!(
                    "commit {} não é um sha completo de 40 hex (ref móvel, branch, tag ou abreviado)",
                    repr_str(commit)
                ),
                "contexto do momento (commit)",
            )],
            "o plugin tem de ser fixado num sha exato, o da prévia que o dono viu".into(),
        ));
    }
    let mut bad: Vec<String> = Vec::new();
    if op == "add" {
        let Staged { dir, preview } = staged(ctx, ce)?;
        let head = git_txt(ce, &dir, &["rev-parse", "HEAD"])?;
        evid.push(ev(
            format!(
                "HEAD do clone em prévia = {}, commit pedido = {}, commit da prévia = {}",
                short(&head),
                short(commit),
                short(&py_str(preview.get("commit")))
            ),
            format!(
                "git rev-parse HEAD em {dir}; {}/preview.json",
                dirname(&dir)
            ),
        ));
        let da_previa = preview.get("commit").and_then(Value::as_str) == Some(commit.as_str());
        if !(head == *commit && da_previa) {
            bad.push("o commit pedido não é o da prévia nem o do clone".into());
        }
        let dirty = git_txt(ce, &dir, &["status", "--porcelain"])?;
        evid.push(ev(
            if dirty.is_empty() {
                "clone em prévia limpo".into()
            } else {
                format!(
                    "clone em prévia com mudança solta: {}",
                    primeira_linha(&dirty)
                )
            },
            format!("git status --porcelain em {dir}"),
        ));
        if !dirty.is_empty() {
            bad.push("há mudança solta no clone: o que executa não é o commit".into());
        }
    } else if op == "update" {
        let pack = installed_pack(ctx, ce)?;
        let upstream = git_txt(ce, &pack, &["rev-parse", "@{u}"])?;
        evid.push(ev(
            format!(
                "ponta do remote rastreado = {}, commit pedido = {}",
                short(&upstream),
                short(commit)
            ),
            format!("git rev-parse @{{u}} em {pack}"),
        ));
        if upstream != *commit {
            bad.push("o commit pedido não é a ponta que a prévia mostrou".into());
        }
        let seen = audited(ce)?.iter().any(|a| {
            get(a, "op") == Some("update-preview")
                && get(a, "plugin") == Some(plugin.as_str())
                && get(a, "target") == Some(commit.as_str())
        });
        evid.push(ev(
            format!(
                "prévia de update com target={} no log: {}",
                short(commit),
                if seen { "sim" } else { "não" }
            ),
            texto(&audit_file(ce)),
        ));
        if !seen {
            bad.push("o dono não viu a prévia deste commit".into());
        }
        let dirty = git_txt(ce, &pack, &["status", "--porcelain"])?;
        if !dirty.is_empty() {
            evid.push(ev(
                format!("pacote com mudança local: {}", primeira_linha(&dirty)),
                format!("git status --porcelain em {pack}"),
            ));
            bad.push("o pacote instalado tem mudança local".into());
        }
    } else {
        let pack = installed_pack(ctx, ce)?;
        let head = git_txt(ce, &pack, &["rev-parse", "HEAD"])?;
        evid.push(ev(
            format!(
                "HEAD do pacote = {}, commit pedido = {}",
                short(&head),
                short(commit)
            ),
            format!("git rev-parse HEAD em {pack}"),
        ));
        if head != *commit {
            bad.push("o commit pedido não é o HEAD do pacote".into());
        }
        let dirty = git_txt(ce, &pack, &["status", "--porcelain"])?;
        evid.push(ev(
            if dirty.is_empty() {
                "pacote limpo".into()
            } else {
                format!("pacote com mudança local: {}", primeira_linha(&dirty))
            },
            format!("git status --porcelain em {pack}"),
        ));
        if !dirty.is_empty() {
            bad.push("o pacote tem mudança local: o que executa não é o commit".into());
        }
        let log = audited(ce)?;
        let last = log.iter().rfind(|a| {
            get(a, "plugin") == Some(plugin.as_str())
                && matches!(get(a, "op"), Some("add" | "update"))
        });
        let mark = last
            .and_then(|a| {
                get(a, "target")
                    .filter(|t| !t.is_empty())
                    .or_else(|| get(a, "commit"))
            })
            .filter(|m| !m.is_empty());
        evid.push(ev(
            format!(
                "último commit instalado no log = {}",
                mark.map_or_else(|| "nenhum".into(), short)
            ),
            texto(&audit_file(ce)),
        ));
        if let Some(m) = mark.filter(|m| *m != head) {
            bad.push(format!(
                "o pacote mudou depois da última instalação registrada ({})",
                short(m)
            ));
        }
        if mark.is_none() {
            let detalhe = if bad.is_empty() {
                "sem registro de instalação: o commit não pode ser conferido com a prévia".into()
            } else {
                bad.join("; ")
            };
            return Ok((if bad.is_empty() { Aviso } else { Barra }, evid, detalhe));
        }
    }
    Ok(if bad.is_empty() {
        (Passa, evid, format!("{} é o sha da prévia", short(commit)))
    } else {
        (Barra, evid, bad.join("; "))
    })
}

// ---------------------------------------------------------------- guarda: executavel

struct Fontes<'a> {
    old: Option<Tree<'a>>,
    next: Tree<'a>,
    onde: String,
}

/// `sourceTrees` (`src/lib/cerberus-limiar.ts:335`): a árvore antiga (ou nenhuma), a nova e onde.
fn source_trees<'a>(ctx: &LimiarCtx, ce: &'a CheckEnv) -> R<Fontes<'a>> {
    if !is_sha(&ctx.commit) {
        return Err(Falha(format!(
            "commit {} não é sha: sem o que comparar (a guarda commit-fixado barra)",
            repr_str(&ctx.commit)
        )));
    }
    if ctx.op == "add" {
        let dir = staged(ctx, ce)?.dir;
        return Ok(Fontes {
            old: None,
            next: Tree::new(ce, &dir, &ctx.commit)?,
            onde: dir,
        });
    }
    let pack = installed_pack(ctx, ce)?;
    let current = git_txt(ce, &pack, &["rev-parse", "HEAD"])?;
    let old = Tree::new(ce, &pack, &current)?;
    let next = if ctx.op == "update" {
        Tree::new(ce, &pack, &ctx.commit)?
    } else {
        Tree::new(ce, &pack, &current)?
    };
    Ok(Fontes {
        old: Some(old),
        next,
        onde: pack,
    })
}

/// `previewRuns` (`src/lib/cerberus-limiar.ts:348`).
fn preview_runs(preview: &Map<String, Value>) -> R<IndexMap<String, String>> {
    let mut out = IndexMap::new();
    for run in preview
        .get("runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(r) = run.as_object().filter(|r| r.contains_key("path")) else {
            return Err(Falha("'path'".into()));
        };
        if !r.contains_key("sha256") {
            return Err(Falha("'sha256'".into()));
        }
        out.insert(py_str(r.get("path")), py_str(r.get("sha256")));
    }
    Ok(out)
}

fn symmetric_diff(a: &BTreeSet<String>, b: &BTreeSet<String>) -> Vec<String> {
    a.difference(b).chain(b.difference(a)).cloned().collect()
}

/// `addedByPreview` (`src/lib/cerberus-limiar.ts:361`).
fn added_by_preview(preview: &Map<String, Value>) -> R<(BTreeSet<String>, BTreeSet<String>)> {
    let vazio = Map::new();
    let adds = as_dict(preview.get("adds")).unwrap_or(&vazio);
    let mut pages = BTreeSet::new();
    for page in adds
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(p) = page.as_object().filter(|p| p.contains_key("id")) else {
            return Err(Falha("'id'".into()));
        };
        pages.insert(py_str(p.get("id")));
    }
    let routes = adds
        .get("routes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|r| py_str(Some(r)))
        .collect();
    Ok((pages, routes))
}

/// `executavelNovo` (`src/lib/cerberus-limiar.ts:372`).
fn executavel_novo(
    ctx: &LimiarCtx,
    ce: &CheckEnv,
    next: &Tree,
    now: &Manifest,
    onde: &str,
) -> R<Verdict> {
    let Staged { preview, .. } = staged(ctx, ce)?;
    let prev = preview_runs(&preview)?;
    let prefix = next.env_prefix()?.unwrap_or_default();
    let listed: BTreeSet<String> = preview
        .get("other")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|v| py_str(Some(v)))
        .collect();
    let mut bad: Vec<String> = Vec::new();
    for rel in sorted(now.files.keys()) {
        let path = format!("{prefix}{rel}");
        if in_preview(&rel) {
            match prev.get(&path) {
                None => bad.push(format!("executável fora da prévia: {path}")),
                Some(sha) if *sha != sha256_of(next, &rel)? => {
                    bad.push(format!("executável diferente da prévia: {path}"))
                }
                Some(_) => {}
            }
        } else if !listed.contains(&path) {
            bad.push(format!("script fora da prévia: {path}"));
        }
    }
    let (pages, routes) = added_by_preview(&preview)?;
    let mut extra = symmetric_diff(&pages, &now.pages);
    extra.extend(symmetric_diff(&routes, &now.routes));
    let extra = sorted(&extra);
    if !extra.is_empty() {
        let nomes: Vec<String> = extra.into_iter().take(NAMED_MAX).collect();
        bad.push(format!(
            "páginas ou rotas do plugin.json diferentes da prévia: {}",
            nomes.join(", ")
        ));
    }
    let mut evid = vec![ev(
        format!(
            "{} arquivo(s) executável(is) no commit {}, {} na prévia; páginas: {}; rotas: {}",
            now.files.len(),
            short(&next.commit),
            prev.len(),
            or_dash(&sorted(&now.pages)),
            or_dash(&sorted(&now.routes))
        ),
        format!(
            "git ls-tree -r {} em {onde}; {}/preview.json",
            short(&next.commit),
            dirname(onde)
        ),
    )];
    evid.extend(bad.iter().map(|b| ev(b, "comparação com preview.json")));
    Ok(if bad.is_empty() {
        (
            Passa,
            evid,
            "o que executa é exatamente o que a prévia mostrou".into(),
        )
    } else {
        (Barra, evid, bad.join("; "))
    })
}

/// `executavelMudado` (`src/lib/cerberus-limiar.ts:394`).
fn executavel_mudado(
    ce: &CheckEnv,
    old: &Tree,
    next: &Tree,
    now: &Manifest,
    onde: &str,
) -> R<Verdict> {
    let before = manifest(old)?;
    let added = sorted(now.files.keys().filter(|r| !before.files.contains_key(*r)));
    let removed = sorted(before.files.keys().filter(|r| !now.files.contains_key(*r)));
    let changed = sorted(
        now.files
            .iter()
            .filter(|(r, b)| before.files.get(*r).is_some_and(|ob| ob != *b))
            .map(|(r, _)| r),
    );
    let mut prefixes: Vec<String> = [old.env_prefix()?, next.env_prefix()?]
        .into_iter()
        .flatten()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    prefixes.sort_by(|a, b| js_cmp(a, b));
    let stat = if old.commit != next.commit {
        let mut args = vec![
            "diff",
            "--stat",
            "--no-color",
            old.commit.as_str(),
            next.commit.as_str(),
            "--",
        ];
        if prefixes.is_empty() {
            args.push(".");
        } else {
            args.extend(prefixes.iter().map(String::as_str));
        }
        git_txt(ce, onde, &args)?
    } else {
        String::new()
    };
    let summary = split_lines(&stat)
        .pop()
        .map_or_else(|| "sem mudança".to_owned(), |l| js_trim(&l).to_owned());
    let mut evid = vec![ev(
        format!(
            "{} → {}: {} executável(is) novo(s), {} alterado(s), {} removido(s); {summary}",
            short(&old.commit),
            short(&next.commit),
            added.len(),
            changed.len(),
            removed.len()
        ),
        format!(
            "git diff --stat {} {} em {onde}",
            short(&old.commit),
            short(&next.commit)
        ),
    )];
    for (tag, list) in [
        ("novo", &added),
        ("alterado", &changed),
        ("removido", &removed),
    ] {
        for rel in list.iter().take(LISTED_MAX) {
            let fora = if in_preview(rel) {
                ""
            } else {
                " (fora do diff da prévia)"
            };
            evid.push(ev(
                format!("{tag}: {rel}{fora}"),
                format!("env/ do commit {}", short(&next.commit)),
            ));
        }
    }
    for (label, a, b) in [
        ("páginas", &before.pages, &now.pages),
        ("rotas", &before.routes, &now.routes),
        ("ações", &before.actions, &now.actions),
    ] {
        if a != b {
            let plus = sorted(b.difference(a));
            let minus = sorted(a.difference(b));
            evid.push(ev(
                format!("{label}: +{} {MINUS}{}", or_dash(&plus), or_dash(&minus)),
                format!("plugin.json de {}", short(&next.commit)),
            ));
        }
    }
    let bad: Vec<String> = added
        .iter()
        .filter(|r| !in_preview(r))
        .map(|r| format!("executável novo fora do diff que a prévia mostra: {r}"))
        .collect();
    let warn: Vec<String> = changed.iter().filter(|r| !in_preview(r)).cloned().collect();
    if !bad.is_empty() {
        return Ok((Barra, evid, bad.join("; ")));
    }
    if !warn.is_empty() {
        let nomes: Vec<String> = warn.into_iter().take(NAMED_MAX).collect();
        return Ok((
            Aviso,
            evid,
            format!(
                "script alterado fora do diff da prévia (só o nome aparece no resumo): {}",
                nomes.join(", ")
            ),
        ));
    }
    let detalhe = if added.is_empty() && changed.is_empty() && removed.is_empty() {
        "nenhum código executável muda"
    } else {
        "o que muda em código executável está no diff da prévia"
    };
    Ok((Passa, evid, detalhe.into()))
}

/// `executavel` (`src/lib/cerberus-limiar.ts:429`).
fn executavel(ctx: &LimiarCtx, ce: &CheckEnv) -> R<Verdict> {
    let Fontes { old, next, onde } = source_trees(ctx, ce)?;
    let now = manifest(&next)?;
    match old {
        Some(old) if ctx.op != "add" => executavel_mudado(ce, &old, &next, &now, &onde),
        _ => executavel_novo(ctx, ce, &next, &now, &onde),
    }
}

// ---------------------------------------------------------------- guarda: invariante

fn toca_cerberus(path: &str) -> bool {
    path.split('/')
        .any(|seg| seg.to_ascii_lowercase().contains("cerberus"))
}

/// `touchesCore` (`src/lib/cerberus-limiar.ts:437`).
fn touches_core(tree: &Tree, skill: &Path) -> Vec<Evid> {
    let mut bad = Vec::new();
    for path in sorted(tree.files.keys()) {
        if toca_cerberus(&path) || path.starts_with("loop/cerberus/") {
            bad.push(ev(
                "arquivo do pacote no nome ou no lugar do Cerberus (guarda, bloco ou registro de cabeça)",
                &path,
            ));
        } else if path.starts_with("loop/") && is_file(&skill.join(&path)) {
            bad.push(ev(
                "o pacote traz arquivo que já é do núcleo (loop/): substituiria o núcleo",
                &path,
            ));
        }
    }
    bad
}

/// `overridesKit` (`src/lib/cerberus-limiar.ts:449`).
fn overrides_kit(
    tree: &Tree,
    skill: &Path,
    prefix: &str,
    env: &IndexMap<String, String>,
) -> R<Vec<Evid>> {
    let mut bad = Vec::new();
    for rel in sorted(env.keys()) {
        let top = topo(&rel);
        if PLUGIN_FILES.contains(&top)
            || NOT_COPIED.contains(&top)
            || NOT_COPIED.contains(&rel.as_str())
        {
            continue;
        }
        if is_file(&skill.join("kit").join(&rel)) {
            bad.push(ev(
                format!(
                    "env/{rel} cai por cima de kit/{rel} no install.sh (rsync do env/ sobre o $D)"
                ),
                format!("{prefix}{rel}"),
            ));
        }
    }
    if env.contains_key("config.tsv") {
        let body = tree.read(&format!("{prefix}config.tsv"), SCAN_MAX)?;
        for ln in split_lines(&String::from_utf8_lossy(&body)) {
            let key = ln.split('\t').next().unwrap_or("");
            if key.starts_with("HADES_CERBERUS_") {
                bad.push(ev(
                    format!("config.tsv do plugin declara {key}: os modos do Cerberus são do dono"),
                    format!("{prefix}config.tsv"),
                ));
            }
        }
    }
    Ok(bad)
}

/// `scanScript` (`src/lib/cerberus-limiar.ts:466`).
fn scan_script(text: &str, onde: &str, bad: &mut Vec<Evid>, warn: &mut Vec<Evid>) {
    let hooky = text.to_ascii_lowercase().contains("hook");
    for (i, ln) in split_lines(text).iter().enumerate() {
        let code = js_trim(ln);
        if code.is_empty() || code.starts_with('#') || code.starts_with("//") {
            continue;
        }
        let at = format!("{onde}:{}", i + 1);
        let lower = ln.to_ascii_lowercase();
        for (what, rx) in TAMPER.iter() {
            if rx.is_match(&lower) {
                bad.push(ev(format!("{what}: {}", js_slice(code, SNIPPET)), &at));
            }
        }
        if SETTINGS.is_match(&lower) {
            let nota = if hooky {
                " (onde o hook é registrado)"
            } else {
                ""
            };
            let e = ev(
                format!(
                    "mexe em settings.json{nota}: {}",
                    js_slice(code, SNIPPET_SETTINGS)
                ),
                &at,
            );
            if hooky { bad.push(e) } else { warn.push(e) }
        }
        let casa = |rx: &FancyRegex| rx.is_match(ln).unwrap_or(false);
        if casa(&SKILL_WRITE_TARGET) && casa(&SKILL_WRITE_ACTION) {
            bad.push(ev(
                format!(
                    "escreve na pasta da skill (o núcleo): {}",
                    js_slice(code, SNIPPET)
                ),
                &at,
            ));
        }
    }
}

/// `invariante` (`src/lib/cerberus-limiar.ts:481`).
fn invariante(ctx: &LimiarCtx, ce: &CheckEnv) -> R<Verdict> {
    if !is_sha(&ctx.commit) {
        return Err(Falha(format!(
            "commit {} não é sha: sem o que varrer (a guarda commit-fixado barra)",
            repr_str(&ctx.commit)
        )));
    }
    let dir = if ctx.op == "add" {
        staged(ctx, ce)?.dir
    } else {
        installed_pack(ctx, ce)?
    };
    let skill = skill_de(ce);
    let tree = Tree::new(ce, &dir, &ctx.commit)?;
    let prefix = tree.env_prefix()?.unwrap_or_default();
    let env = tree.env_files()?;
    let mut bad = touches_core(&tree, &skill);
    bad.extend(overrides_kit(&tree, &skill, &prefix, &env)?);
    let mut warn = Vec::new();
    if manifest(&tree)?.pages.contains("cerberus") {
        bad.push(ev(
            "plugin.json registra a página \"cerberus\", que é do núcleo",
            format!("{prefix}plugin.json"),
        ));
    }
    let scripts: Vec<String> = sorted(env.keys())
        .into_iter()
        .filter(|r| is_exec(r))
        .collect();
    for rel in &scripts {
        let body = tree.read(&format!("{prefix}{rel}"), SCAN_MAX)?;
        scan_script(
            &String::from_utf8_lossy(&body),
            &format!("{prefix}{rel}"),
            &mut bad,
            &mut warn,
        );
    }
    if !bad.is_empty() {
        let n = bad.len();
        bad.truncate(EVID_MAX);
        return Ok((Barra, bad, format!("{n} achado(s) contra o núcleo")));
    }
    if !warn.is_empty() {
        warn.truncate(EVID_MAX);
        return Ok((Aviso, warn, "o plugin mexe em settings.json".into()));
    }
    Ok((
        Passa,
        vec![ev(
            format!(
                "{} arquivo(s) do pacote e {} executável(is) varridos: nada cai sobre o núcleo (kit/, loop/), o hook hades-guard, as guardas nem os modos HADES_CERBERUS_*",
                tree.files.len(),
                scripts.len()
            ),
            format!("git ls-tree -r {} em {dir}", short(&ctx.commit)),
        )],
        "sem sobrescrita de invariante do núcleo".into(),
    ))
}

// ---------------------------------------------------------------- guarda: ensaio

const ENSAIO_RUNS: [&str; 3] = ["post-install.sh", "doctor.sh", "checks/*.sh"];
/// Arquivo maior que isto não é extraído para o ensaio (assets e afins).
const ENSAIO_FILE_MAX: usize = 2_000_000;

/// `ensaioInput` (`src/lib/cerberus-limiar.ts:510`): o `env/` do commit pronto para o ensaio, os scripts a rodar e
/// os arquivos ao lado deles (sem assets, reference, `__pycache__`).
fn ensaio_input(tree: &Tree) -> R<EnsaioIn> {
    let prefix = tree.env_prefix()?.unwrap_or_default();
    let env = tree.env_files()?;
    let mut inp = EnsaioIn::default();
    for rel in sorted(env.keys()) {
        if ENV_SKIP.contains(&topo(&rel)) {
            continue;
        }
        let path = format!("{prefix}{rel}");
        let body = tree.read(&path, ENSAIO_FILE_MAX + 1)?;
        if body.len() > ENSAIO_FILE_MAX {
            continue;
        }
        let exec = tree.files.get(&path).is_some_and(|e| e.mode == "100755");
        if ENSAIO_RUNS.iter().any(|pat| fnmatch_case(&rel, pat)) {
            inp.scripts
                .insert(rel.clone(), String::from_utf8_lossy(&body).into_owned());
        }
        inp.files.insert(
            rel,
            Arquivo {
                body,
                mode: if exec { 0o755 } else { 0o644 },
            },
        );
    }
    if let Some(pj) = env.get("plugin.json") {
        let manifest = parse_json(
            &tree.blob_text(pj)?,
            &format!("plugin.json do commit {} ilegível", short(&tree.commit)),
        )?;
        inp.rede = rede_declarada(&manifest);
        inp.escrita = escrita_declarada(&manifest);
    }
    Ok(inp)
}

/// `ensaio` (`src/lib/cerberus-limiar.ts:527`).
fn ensaio(ctx: &LimiarCtx, ce: &CheckEnv) -> R<Verdict> {
    let fontes = source_trees(ctx, ce)?;
    let out = ensaiar(&ensaio_input(&fontes.next)?, ce)?;
    Ok((out.resultado, out.evidencia, out.detalhe))
}

// ---------------------------------------------------------------- registro e borda

/// `LIMIAR_GUARDS` (`src/lib/cerberus-limiar.ts:533`).
const GUARDAS: [(&str, Guard); 4] = [
    ("commit-fixado", commit_fixado),
    ("executavel", executavel),
    ("invariante", invariante),
    ("ensaio", ensaio),
];

/// `rodarLimiar` (`src/lib/cerberus-limiar.ts:533`): o veredito da guarda; qualquer falha ao conferir vira barra com
/// o que faltou. `None` para guarda desconhecida.
pub fn rodar(guarda: &str, ce: &CheckEnv) -> Option<CheckVerdict> {
    let run = GUARDAS
        .iter()
        .find(|(n, _)| *n == guarda)
        .map(|(_, g)| *g)?;
    let (resultado, mut evidencia, detalhe) = context(ce)
        .and_then(|ctx| run(&ctx, ce))
        .unwrap_or_else(|Falha(why)| {
            (
                Barra,
                vec![ev(
                    format!("não consegui conferir: {why}"),
                    format!("checks/limiar.py {guarda}"),
                )],
                "falha ao conferir".into(),
            )
        });
    evidencia.truncate(EVID_MAX);
    Some(CheckVerdict {
        resultado,
        evidencia,
        detalhe: Some(js_slice(&detalhe, DETAIL_MAX)),
    })
}

#[cfg(test)]
mod tests;
