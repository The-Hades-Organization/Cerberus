//! Ensaio isolado do código executável de um plugin (`src/lib/cerberus-ensaio.ts`), a guarda `plugin-ensaio-isolado`
//! do Limiar: cada script roda num bubblewrap com a raiz somente leitura, HOME em tmpfs somente leitura, uma cópia
//! descartável do `$D` gravável no caminho do `$D` e sem rede (salvo declaração no `plugin.json`). O `$D` real nunca é
//! tocado e o `docker.sock` nunca é montado: `connect()` num socket Unix não é barrado por montagem somente leitura nem
//! por `--unshare-net`, então `/run`, `/tmp`, `/var/tmp` e o `XDG_RUNTIME_DIR` viram tmpfs vazios e cada socket achado
//! em outros pontos do host é coberto com `/dev/null`. Escrita no worktree só por declaração (`ensaio.escrita[]`), e
//! sobre uma cópia descartável do caminho declarado.

use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fancy_regex::Regex as FancyRegex;
use indexmap::IndexMap;
use regex::Regex;
use serde_json::Value;

use crate::check::{CheckEnv, Evid, ev};
use crate::limiar::JS_S;
use crate::modelo::{Resultado, js_trim};

const SCRIPT_TIMEOUT_S: u64 = 20;
const BUDGET_S: u64 = 240;
const OUTPUT_MAX: usize = 1 << 20;
const PATHS_NAMED: usize = 3;
const TAIL: usize = 160;
const SOCKET_SCAN_ROOTS: [&str; 6] = ["/etc", "/var", "/opt", "/srv", "/mnt", "/media"];
const SOCKET_SCAN_DEPTH: &str = "6";
const SOCKET_SCAN_TIMEOUT: Duration = Duration::from_secs(10);
const BWRAP_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const COPY_SKIP: [&str; 4] = ["logs", "plugins-staging", "_maquina", ".git"];
const INSTALL_HINT: &str = "instale o pacote bubblewrap da sua distribuição (ex.: pacman -S bubblewrap · apt install bubblewrap · dnf install bubblewrap)";
const DOCTOR_WRAPPER: &str = "ok(){ :; }; warn(){ :; }; err(){ :; }; have(){ command -v \"$1\" >/dev/null 2>&1; }; G=Projeto; QUICK=1; D=$HADES_DATA; W=${HADES_WT:-}; source \"$1\"";
const POLL: Duration = Duration::from_millis(5);

/// `WRITE_DENIED` (`src/lib/cerberus-ensaio.ts:23`).
static WRITE_DENIED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("(?i)read-only file system").expect("regex"));
/// `NET_MARKERS` (`src/lib/cerberus-ensaio.ts:24`).
static NET_MARKERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        "(?i)network is unreachable|network is down|could not resolve|name or service not known|temporary failure in name resolution\
         |failed to connect|connection refused|no route to host|unable to resolve|couldn't connect",
    )
    .expect("regex")
});
/// `DOCKER_CALL` (`src/lib/cerberus-ensaio.ts:30`): docker chamado como comando.
static DOCKER_CALL: LazyLock<FancyRegex> = LazyLock::new(|| {
    FancyRegex::new(&format!(
        r"(?:^|[{JS_S};\&|(`$])(?:sudo[{JS_S}]+)?docker(?:-compose)?(?=[{JS_S}]|$)"
    ))
    .expect("regex")
});
/// `DOCKER_PROBE` (`src/lib/cerberus-ensaio.ts:31`): `command -v docker`, `have docker` e afins só sondam a presença.
static DOCKER_PROBE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?:command[{JS_S}]+-v|which|type[{JS_S}]+-p|type|have|hash)[{JS_S}]+docker"
    ))
    .expect("regex")
});
static QUOTED_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"'(/[^']+)'").expect("regex"));
static PATH_BEFORE_RO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"(?i)(/[^{JS_S}:'"]+)[{JS_S}]*:[{JS_S}]*read-only"#
    ))
    .expect("regex")
});

/// `Escrita` (`src/lib/cerberus-ensaio.ts:35`): caminho do worktree que um item pode gravar, com o motivo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Escrita {
    pub caminho: String,
    pub motivo: String,
}

/// `Estado` (`src/lib/cerberus-ensaio.ts:36`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Estado {
    Ensaiado,
    Barrado,
    NaoEnsaiado,
}

/// `Resultado` de um script (`src/lib/cerberus-ensaio.ts:37`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ensaiado {
    pub rel: String,
    pub estado: Estado,
    pub motivo: String,
    pub nota: Option<String>,
}

/// Um arquivo do `env/` a extrair ao lado dos scripts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arquivo {
    pub body: Vec<u8>,
    pub mode: u32,
}

/// `EnsaioIn` (`src/lib/cerberus-ensaio.ts:38`).
#[derive(Clone, Debug, Default)]
pub struct EnsaioIn {
    /// Scripts a ensaiar, caminho relativo ao `env/` → conteúdo.
    pub scripts: IndexMap<String, String>,
    /// Todo o `env/` do plugin a extrair ao lado dos scripts.
    pub files: IndexMap<String, Arquivo>,
    /// Item → motivo, do campo `ensaio.rede` do `plugin.json`.
    pub rede: IndexMap<String, String>,
    /// Item → caminhos do worktree que ele pode gravar, do campo `ensaio.escrita` do `plugin.json`.
    pub escrita: IndexMap<String, Vec<Escrita>>,
}

/// `EnsaioOut` (`src/lib/cerberus-ensaio.ts:231`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnsaioOut {
    pub resultado: Resultado,
    pub evidencia: Vec<Evid>,
    pub detalhe: String,
}

fn is_dir(p: &Path) -> bool {
    !p.as_os_str().is_empty() && fs::metadata(p).is_ok_and(|m| m.is_dir())
}

fn lista_de<'a>(plugin_json: &'a Value, campo: &str) -> Option<&'a Vec<Value>> {
    plugin_json
        .get("ensaio")?
        .as_object()?
        .get(campo)?
        .as_array()
}

fn texto_de<'a>(entry: &'a Value, campo: &str) -> Option<&'a str> {
    entry.as_object()?.get(campo)?.as_str()
}

/// `redeDeclarada` (`src/lib/cerberus-ensaio.ts:52`): o campo opcional `ensaio.rede[]` do `plugin.json`,
/// `{item, motivo}`; entrada sem item ou sem motivo não vale.
pub fn rede_declarada(plugin_json: &Value) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    for entry in lista_de(plugin_json, "rede").into_iter().flatten() {
        let (Some(item), Some(motivo)) = (texto_de(entry, "item"), texto_de(entry, "motivo"))
        else {
            continue;
        };
        let motivo = js_trim(motivo);
        if !item.is_empty() && !motivo.is_empty() {
            out.insert(item.to_owned(), motivo.to_owned());
        }
    }
    out
}

/// `path.posix.normalize` do Node.
fn normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    let mut out = parts.join("/");
    if out.is_empty() && !absolute {
        out = ".".into();
    }
    if !out.is_empty() && trailing {
        out.push('/');
    }
    if absolute { format!("/{out}") } else { out }
}

/// `escritaDeclarada` (`src/lib/cerberus-ensaio.ts:65`): o campo opcional `ensaio.escrita[]`, `{item, caminho, motivo}`;
/// caminho relativo ao worktree e contido nele, sem `..`.
pub fn escrita_declarada(plugin_json: &Value) -> IndexMap<String, Vec<Escrita>> {
    let mut out: IndexMap<String, Vec<Escrita>> = IndexMap::new();
    for entry in lista_de(plugin_json, "escrita").into_iter().flatten() {
        let (Some(item), Some(caminho), Some(motivo)) = (
            texto_de(entry, "item"),
            texto_de(entry, "caminho"),
            texto_de(entry, "motivo"),
        ) else {
            continue;
        };
        let motivo = js_trim(motivo);
        if item.is_empty() || motivo.is_empty() {
            continue;
        }
        let rel = normalize(caminho).trim_end_matches('/').to_owned();
        if rel.is_empty()
            || rel == "."
            || rel.starts_with('/')
            || rel == ".."
            || rel.starts_with("../")
        {
            continue;
        }
        out.entry(item.to_owned()).or_default().push(Escrita {
            caminho: rel,
            motivo: motivo.to_owned(),
        });
    }
    out
}

/// `findBwrap` (`src/lib/cerberus-ensaio.ts:80`): o primeiro `bwrap` que é arquivo no `PATH` dado.
pub fn find_bwrap(path: Option<&str>) -> Option<PathBuf> {
    path.unwrap_or("")
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join("bwrap"))
        .find(|c| fs::metadata(c).is_ok_and(|m| m.is_file()))
}

/// `callsDocker` (`src/lib/cerberus-ensaio.ts:89`).
fn calls_docker(text: &str) -> bool {
    text.split('\n').any(|ln| {
        let code = js_trim(ln);
        !code.is_empty()
            && !code.starts_with('#')
            && DOCKER_CALL.is_match(code).unwrap_or(false)
            && !DOCKER_PROBE.is_match(code)
    })
}

/// `copyTree` (`src/lib/cerberus-ensaio.ts:97`): cópia descartável de uma pasta (sem o que é volumoso no topo); só
/// arquivos, pastas e links, que ficam como estão: socket e fifo ficam de fora.
fn copy_tree(from: &Path, dest: &Path, skip_top: &[&str]) -> io::Result<()> {
    if !is_dir(from) {
        return Ok(());
    }
    copy_dir(from, dest, skip_top)
}

fn copy_dir(from: &Path, dest: &Path, skip: &[&str]) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip.iter().any(|s| name.as_encoded_bytes() == s.as_bytes()) {
            continue;
        }
        let (src, to) = (entry.path(), dest.join(&name));
        let kind = fs::symlink_metadata(&src)?.file_type();
        if kind.is_symlink() {
            let _ = fs::remove_file(&to);
            symlink(fs::read_link(&src)?, &to)?;
        } else if kind.is_dir() {
            copy_dir(&src, &to, &[])?;
        } else if kind.is_file() {
            fs::copy(&src, &to)?;
        }
    }
    fs::set_permissions(dest, fs::metadata(from)?.permissions())
}

struct WriteBind {
    copy: PathBuf,
    target: PathBuf,
}

/// `declaredTarget` (`src/lib/cerberus-ensaio.ts:113`): caminho declarado resolvido dentro do worktree (links não
/// escapam); sem worktree, sem caminho ou fora dele, nada.
fn declared_target(worktree: &str, caminho: &str) -> Option<PathBuf> {
    if worktree.is_empty() {
        return None;
    }
    let base = fs::canonicalize(worktree).ok()?;
    let target = fs::canonicalize(Path::new(worktree).join(caminho)).ok()?;
    (is_dir(&target) && target.starts_with(&base) && target != base).then_some(target)
}

/// `writtenPaths` (`src/lib/cerberus-ensaio.ts:121`): caminhos que o script tentou gravar, o que vem ao lado de
/// "Read-only file system" na saída de erro.
fn written_paths(output: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for ln in output.split('\n') {
        if !WRITE_DENIED.is_match(ln) {
            continue;
        }
        let hit = QUOTED_PATH
            .captures(ln)
            .or_else(|| PATH_BEFORE_RO.captures(ln))
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_owned());
        if let Some(p) = hit
            && !found.contains(&p)
        {
            found.push(p);
        }
    }
    found
}

/// `text.slice(-n)` do JS (unidades UTF-16), sem partir caractere.
fn js_slice_end(text: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in text.char_indices().rev() {
        units += c.len_utf16();
        if units > n {
            return &text[i + c.len_utf8()..];
        }
    }
    text
}

/// `tail` (`src/lib/cerberus-ensaio.ts:131`): as duas últimas linhas, juntas por ` ⏎ `, com no máximo 160 unidades.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = js_trim(text).split('\n').collect();
    let last = lines[lines.len().saturating_sub(2)..].join(" ⏎ ");
    js_slice_end(&last, TAIL).to_owned()
}

/// `Sandbox` (`src/lib/cerberus-ensaio.ts:133`).
struct Sandbox {
    bwrap: PathBuf,
    copy: PathBuf,
    target: PathBuf,
    env: Vec<(String, String)>,
    home: String,
    worktree: String,
    skill: String,
    /// Pastas que viram tmpfs vazio (e somente leitura): onde o host guarda sockets de daemons.
    volatile: Vec<String>,
    /// Sockets fora dessas pastas, cobertos com `/dev/null`.
    sockets: Vec<String>,
}

fn real(p: &str) -> Option<String> {
    fs::canonicalize(p)
        .ok()
        .map(|r| r.to_string_lossy().into_owned())
}

/// `volatileDirs` (`src/lib/cerberus-ensaio.ts:141`): pastas do host que somem no sandbox: `/run` (docker.sock,
/// systemd, dbus), `/tmp`, `/var/tmp` e o `XDG_RUNTIME_DIR` (barramento da sessão).
fn volatile_dirs(xdg: Option<&str>) -> Vec<String> {
    let mut dirs: Vec<String> = vec!["/run".into(), "/tmp".into(), "/var/tmp".into()];
    if let Some(xdg) = xdg.filter(|x| x.starts_with('/') && is_dir(Path::new(x)))
        && let Some(r) = real(xdg)
        && !dirs.contains(&r)
    {
        dirs.push(r);
    }
    dirs.retain(|d| !d.starts_with("/run/") && !d.starts_with("/tmp/"));
    dirs.retain(|d| is_dir(Path::new(d)));
    dirs
}

/// `findSockets` (`src/lib/cerberus-ensaio.ts:150`): sockets Unix fora das pastas que somem, um a um, pois não dá
/// para escondê-los por montagem de pasta. `find` sai com 1 quando perde permissão numa pasta: o que listou vale.
fn find_sockets(extra_roots: &[&str]) -> Vec<String> {
    let roots: Vec<&str> = SOCKET_SCAN_ROOTS
        .iter()
        .copied()
        .chain(extra_roots.iter().copied())
        .filter(|r| is_dir(Path::new(r)))
        .collect();
    if roots.is_empty() {
        return Vec::new();
    }
    let mut args: Vec<String> = roots.iter().map(|r| (*r).to_owned()).collect();
    args.extend(
        [
            "-xdev",
            "-maxdepth",
            SOCKET_SCAN_DEPTH,
            "-type",
            "s",
            "-print0",
        ]
        .map(String::from),
    );
    let mut cmd = Command::new("find");
    cmd.args(&args).stderr(Stdio::null());
    let out = spawn(cmd, SOCKET_SCAN_TIMEOUT);
    out.stdout
        .split('\0')
        .filter(|p| p.starts_with('/') && !p.contains('\n'))
        .map(String::from)
        .collect()
}

/// `resolvConfTargets` (`src/lib/cerberus-ensaio.ts:196`): o `resolv.conf` real quando mora sob `/run`.
fn resolv_conf_targets() -> Vec<String> {
    real("/etc/resolv.conf")
        .filter(|r| r.starts_with("/run/"))
        .into_iter()
        .collect()
}

fn under(path: &str, dir: &str) -> bool {
    path.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
}

/// `bwrapArgs` (`src/lib/cerberus-ensaio.ts:159`).
fn bwrap_args(
    sb: &Sandbox,
    with_net: bool,
    writes: &[WriteBind],
    cwd: &str,
    argv: &[&str],
) -> Vec<String> {
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let mut args: Vec<String> = vec!["--die-with-parent".into(), "--unshare-pid".into()];
    if !with_net {
        args.push("--unshare-net".into());
    }
    args.extend(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"].map(String::from));
    for dir in &sb.volatile {
        args.extend(["--tmpfs".into(), dir.clone()]);
    }
    if with_net {
        for f in resolv_conf_targets() {
            args.extend(["--ro-bind".into(), f.clone(), f]);
        }
    }
    if !sb.home.is_empty() {
        args.extend(["--tmpfs".into(), sb.home.clone()]);
    }
    args.extend(["--bind".into(), s(&sb.copy), s(&sb.target)]);
    let vanished: Vec<&str> = sb
        .volatile
        .iter()
        .map(String::as_str)
        .chain(Some(sb.home.as_str()))
        .filter(|v| !v.is_empty())
        .collect();
    for keep in [&sb.worktree, &sb.skill] {
        if !keep.is_empty() && vanished.iter().any(|v| under(keep, v)) && Path::new(keep).exists() {
            args.extend(["--ro-bind".into(), keep.clone(), keep.clone()]);
        }
    }
    for w in writes {
        args.extend(["--bind".into(), s(&w.copy), s(&w.target)]);
    }
    for sock in &sb.sockets {
        args.extend(["--ro-bind".into(), "/dev/null".into(), sock.clone()]);
    }
    for dir in sb.volatile.iter().chain(Some(&sb.home)) {
        if !dir.is_empty() {
            args.extend(["--remount-ro".into(), dir.clone()]);
        }
    }
    args.extend(["--chdir".into(), cwd.to_owned(), "--".into()]);
    args.extend(argv.iter().map(|a| (*a).to_owned()));
    args
}

/// O que o `spawnSync` do Node devolve: `status` ou `signal`, saídas e se o tempo estourou.
#[derive(Default)]
struct Spawned {
    status: Option<i32>,
    signal: Option<String>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = p.read(&mut chunk) {
                if n == 0 || buf.len() > OUTPUT_MAX {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
        }
        buf
    })
}

/// Nome do sinal como o Node o dá em `r.signal`.
fn signal_name(sig: i32) -> String {
    const NAMES: [&str; 31] = [
        "SIGHUP",
        "SIGINT",
        "SIGQUIT",
        "SIGILL",
        "SIGTRAP",
        "SIGABRT",
        "SIGBUS",
        "SIGFPE",
        "SIGKILL",
        "SIGUSR1",
        "SIGSEGV",
        "SIGUSR2",
        "SIGPIPE",
        "SIGALRM",
        "SIGTERM",
        "SIGSTKFLT",
        "SIGCHLD",
        "SIGCONT",
        "SIGSTOP",
        "SIGTSTP",
        "SIGTTIN",
        "SIGTTOU",
        "SIGURG",
        "SIGXCPU",
        "SIGXFSZ",
        "SIGVTALRM",
        "SIGPROF",
        "SIGWINCH",
        "SIGIO",
        "SIGPWR",
        "SIGSYS",
    ];
    usize::try_from(sig - 1)
        .ok()
        .and_then(|i| NAMES.get(i))
        .map_or_else(|| sig.to_string(), |n| (*n).to_owned())
}

/// `spawnSync` com stdin fechado, tempo limite com SIGKILL no filho e teto de saída (`maxBuffer`, que também mata).
/// Falha ao iniciar dá `status` e `signal` vazios, como o `r.error` do Node.
fn spawn(mut cmd: Command, timeout: Duration) -> Spawned {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped());
    let Ok(mut child) = cmd.spawn() else {
        return Spawned::default();
    };
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {}
            Err(_) => break None,
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill();
            break child.wait().ok();
        }
        thread::sleep(POLL);
    };
    let mut stdout = out.join().unwrap_or_default();
    let mut stderr = err.join().unwrap_or_default();
    let exceeded = stdout.len() > OUTPUT_MAX || stderr.len() > OUTPUT_MAX;
    stdout.truncate(OUTPUT_MAX);
    stderr.truncate(OUTPUT_MAX);
    let (status, signal) = if exceeded || timed_out {
        (None, Some("SIGKILL".to_owned()))
    } else {
        (
            status.and_then(|s| s.code()),
            status.and_then(|s| s.signal()).map(signal_name),
        )
    };
    Spawned {
        status,
        signal,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        timed_out,
    }
}

/// `runOne` (`src/lib/cerberus-ensaio.ts:181`).
fn run_one(
    sb: &Sandbox,
    rel: &str,
    with_net: bool,
    writes: &[WriteBind],
    timeout_s: u64,
) -> Ensaiado {
    let path = format!("{}/.ensaio-env/{rel}", sb.target.to_string_lossy());
    let argv: Vec<&str> = if rel == "doctor.sh" {
        vec!["bash", "-c", DOCTOR_WRAPPER, "_", &path]
    } else {
        vec!["bash", &path]
    };
    let cwd = path.rsplit_once('/').map_or(".", |(d, _)| d);
    let mut cmd = Command::new(&sb.bwrap);
    cmd.args(bwrap_args(sb, with_net, writes, cwd, &argv))
        .env_clear()
        .envs(sb.env.iter().map(|(k, v)| (k, v)))
        .stderr(Stdio::piped());
    let r = spawn(cmd, Duration::from_secs(timeout_s));
    let feito = |estado, motivo: String| Ensaiado {
        rel: rel.to_owned(),
        estado,
        motivo,
        nota: None,
    };
    if r.timed_out {
        return feito(
            Estado::Barrado,
            format!("passou de {timeout_s} s no ensaio"),
        );
    }
    if r.status == Some(0) {
        return feito(Estado::Ensaiado, "saiu com 0".into());
    }
    let output = format!("{}\n{}", r.stderr, r.stdout);
    let paths = written_paths(&output);
    if !paths.is_empty() {
        let named: Vec<&str> = paths.iter().take(PATHS_NAMED).map(String::as_str).collect();
        return feito(
            Estado::Barrado,
            format!(
                "escreve fora do $D (para gravar no worktree, declare em plugin.json → ensaio.escrita): {}",
                named.join(", ")
            ),
        );
    }
    let t = tail(&output);
    if NET_MARKERS.is_match(&output) {
        return feito(
            Estado::Barrado,
            format!("usa rede sem declarar (declare em plugin.json → ensaio.rede): {t}"),
        );
    }
    let code = r
        .status
        .map(|c| c.to_string())
        .or(r.signal)
        .unwrap_or_else(|| "null".into());
    let extra = if t.is_empty() {
        String::new()
    } else {
        format!(" — {t}")
    };
    feito(
        Estado::Barrado,
        format!("falhou no ensaio: saiu com {code}{extra}"),
    )
}

/// `bwrapWorks` (`src/lib/cerberus-ensaio.ts:204`): o bubblewrap sobe aqui (kernel sem user namespace, por exemplo,
/// não sobe)? `None` quando sobe; senão, o porquê.
fn bwrap_works(bwrap: &Path) -> Option<String> {
    let mut cmd = Command::new(bwrap);
    cmd.args(["--unshare-pid", "--ro-bind", "/", "/", "--", "true"])
        .stderr(Stdio::piped());
    let r = spawn(cmd, BWRAP_PROBE_TIMEOUT);
    if r.status == Some(0) {
        return None;
    }
    let t = tail(&r.stderr);
    Some(if t.is_empty() { "não subiu".into() } else { t })
}

/// `line` (`src/lib/cerberus-ensaio.ts:209`).
fn line(r: &Ensaiado, extra: &str) -> Evid {
    let estado = match r.estado {
        Estado::Ensaiado => "ensaiado",
        Estado::Barrado => "barrado",
        Estado::NaoEnsaiado => "não ensaiado",
    };
    let nota = r
        .nota
        .as_deref()
        .map(|n| format!(" — {n}"))
        .unwrap_or_default();
    ev(
        format!("{}: {estado} ({}){nota}{extra}", r.rel, r.motivo),
        format!("bwrap --ro-bind / / … {}", r.rel),
    )
}

/// Pasta de trabalho do ensaio (`mkdtempSync(join(tmpdir(), "hades-ensaio-"))`), apagada ao sair.
struct Work(PathBuf);

impl Work {
    fn new() -> io::Result<Self> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir();
        loop {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos());
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = base.join(format!(
                "hades-ensaio-{:x}{nanos:x}{n:x}",
                std::process::id()
            ));
            match fs::create_dir(&dir) {
                Ok(()) => {
                    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
                    return Ok(Self(dir));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Feito {
    r: Ensaiado,
    rede: Option<String>,
    liberadas: Vec<Escrita>,
}

/// `ensaiar` (`src/lib/cerberus-ensaio.ts:213`): ensaia cada script do `env/` e dá o veredito da guarda. Falha de
/// disco ao montar o sandbox sai como erro, que o Limiar transforma em `barra` com o que faltou.
pub fn ensaiar(inp: &EnsaioIn, ce: &CheckEnv) -> io::Result<EnsaioOut> {
    let mut names: Vec<&String> = inp.scripts.keys().collect();
    names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    if names.is_empty() {
        return Ok(EnsaioOut {
            resultado: Resultado::Ok,
            evidencia: vec![ev(
                "nada a ensaiar: o plugin não tem post-install.sh, doctor.sh nem checks/",
                "env/ do commit",
            )],
            detalhe: "sem código ensaiável".into(),
        });
    }
    let bwrap = find_bwrap(ce.var("PATH"));
    let broken = bwrap.as_deref().and_then(bwrap_works);
    let Some(bwrap) = bwrap.filter(|_| broken.is_none()) else {
        let why = match broken {
            Some(b) => format!("o bubblewrap não subiu aqui ({b})"),
            None => format!("bwrap fora do PATH: {INSTALL_HINT}"),
        };
        let mut evidencia = vec![ev(
            format!("{} script(s) não ensaiado(s): {why}", names.len()),
            "PATH",
        )];
        evidencia.extend(names.iter().map(|n| {
            ev(
                format!("{n}: não ensaiado (sem bubblewrap)"),
                "env/ do commit",
            )
        }));
        return Ok(EnsaioOut {
            resultado: Resultado::Aviso,
            evidencia,
            detalhe: "ensaio não feito: sem bubblewrap utilizável".into(),
        });
    };

    let work = Work::new()?;
    let real_data = ce.var("HADES_DATA").unwrap_or("");
    let copy = work.0.join("D");
    fs::create_dir_all(&copy)?;
    copy_tree(Path::new(real_data), &copy, &COPY_SKIP)?;
    for (rel, arq) in &inp.files {
        let dest = copy.join(".ensaio-env").join(rel);
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&dest, &arq.body)?;
        fs::set_permissions(&dest, fs::Permissions::from_mode(arq.mode))?;
    }
    fs::create_dir_all(copy.join(".ensaio-tmp"))?;
    let target = if is_dir(Path::new(real_data)) {
        PathBuf::from(real_data)
    } else {
        copy.clone()
    };
    let home = ce.var("HOME").unwrap_or("").to_owned();
    let mut env: Vec<(String, String)> = vec![
        ("PATH".into(), ce.var("PATH").unwrap_or("").to_owned()),
        ("HOME".into(), home.clone()),
        (
            "LANG".into(),
            ce.var("LANG").unwrap_or("C.UTF-8").to_owned(),
        ),
        (
            "TMPDIR".into(),
            target.join(".ensaio-tmp").to_string_lossy().into_owned(),
        ),
        ("HADES_DATA".into(), target.to_string_lossy().into_owned()),
    ];
    for key in ["HADES_WT", "HADES_SKILL_DIR"] {
        if let Some(v) = ce.var(key).filter(|v| !v.is_empty()) {
            env.push((key.into(), v.to_owned()));
        }
    }
    let worktree = ce.var("HADES_WT").unwrap_or("").to_owned();
    let skill = ce.var("HADES_SKILL_DIR").unwrap_or("").to_owned();
    let xdg = ce
        .var("XDG_RUNTIME_DIR")
        .map(String::from)
        .or_else(|| std::env::var("XDG_RUNTIME_DIR").ok());
    let sb = Sandbox {
        bwrap,
        copy,
        target,
        env,
        home,
        volatile: volatile_dirs(xdg.as_deref()),
        sockets: find_sockets(&[&worktree, &skill]),
        worktree,
        skill,
    };
    let started = Instant::now();
    let mut results: Vec<Feito> = Vec::new();
    for rel in names {
        let solto = |motivo: String, nota: Option<&str>| Feito {
            r: Ensaiado {
                rel: rel.clone(),
                estado: Estado::NaoEnsaiado,
                motivo,
                nota: nota.map(String::from),
            },
            rede: None,
            liberadas: Vec::new(),
        };
        if calls_docker(&inp.scripts[rel]) {
            results.push(solto(
                "docker".into(),
                Some("o docker.sock nunca entra no sandbox"),
            ));
            continue;
        }
        if started.elapsed().as_secs_f64() > BUDGET_S as f64 {
            results.push(solto(
                format!("passou do orçamento de {BUDGET_S} s do ensaio"),
                None,
            ));
            continue;
        }
        let rede = inp.rede.get(rel).cloned();
        let (mut liberadas, mut writes) = (Vec::new(), Vec::new());
        for (i, e) in inp.escrita.get(rel).into_iter().flatten().enumerate() {
            let Some(target) = declared_target(&sb.worktree, &e.caminho) else {
                continue;
            };
            let copy = work.0.join(format!("W{}-{i}", results.len()));
            copy_tree(&target, &copy, &[])?;
            writes.push(WriteBind { copy, target });
            liberadas.push(e.clone());
        }
        let r = run_one(&sb, rel, rede.is_some(), &writes, SCRIPT_TIMEOUT_S);
        results.push(Feito { r, rede, liberadas });
    }
    drop(work);
    Ok(veredito(&results))
}

fn veredito(results: &[Feito]) -> EnsaioOut {
    let evidencia = results
        .iter()
        .map(|f| {
            let mut extra = String::new();
            if let Some(rede) = &f.rede {
                extra.push_str(&format!("; rede liberada por declaração: {rede}"));
            }
            if !f.liberadas.is_empty() {
                let ditas: Vec<String> = f
                    .liberadas
                    .iter()
                    .map(|e| format!("{} ({})", e.caminho, e.motivo))
                    .collect();
                extra.push_str(&format!(
                    "; escrita liberada por declaração (cópia descartável): {}",
                    ditas.join(", ")
                ));
            }
            line(&f.r, &extra)
        })
        .collect();
    let conta = |e: Estado| results.iter().filter(|f| f.r.estado == e).count();
    let (barrados, feitos) = (conta(Estado::Barrado), conta(Estado::Ensaiado));
    if barrados > 0 {
        return EnsaioOut {
            resultado: Resultado::Barra,
            evidencia,
            detalhe: format!("{barrados} script(s) barrado(s) no ensaio"),
        };
    }
    let com_rede = results.iter().filter(|f| f.rede.is_some()).count();
    let mut detalhe = format!("{feitos} script(s) ensaiado(s)");
    if com_rede > 0 {
        detalhe.push_str(&format!(", {com_rede} com rede liberada por declaração"));
    }
    if feitos < results.len() {
        detalhe.push_str(&format!(", {} não ensaiado(s)", results.len() - feitos));
    }
    EnsaioOut {
        resultado: Resultado::Ok,
        evidencia,
        detalhe,
    }
}

#[cfg(test)]
mod tests;
