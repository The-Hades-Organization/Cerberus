//! Infraestrutura da Vigília (`src/lib/cerberus-vigilia.ts`): leitura dos alvos no registro do orquestrador, blocos do
//! report e o veredito por achados. Cada guarda junta achados (nível, conferido, onde); o veredito é o pior nível.
//! Falha ao conferir vira `aviso` com o que faltou, nunca `ok` calado. As guardas de auditoria moram em
//! [`auditoria`]; as de coerência (registro, plano), em [`coerencia`].

pub mod auditoria;
pub mod coerencia;

use std::cell::OnceCell;
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use hades_core::config::split_lines;
use hades_core::paths::{skill_root, work_root};
use hades_core::proc::{RunOptions, run, which};
use regex::Regex;
use serde_json::{Map, Value};

use crate::Opcoes;
use crate::check::{CheckEnv, CheckVerdict, Evid, js_slice};
use crate::config::{Cfg, config};
use crate::modelo::{
    DETAIL_MAX, EVID_MAX, Resultado, TEXT_MAX, js_norm, js_space, js_string, js_trim,
};

pub use auditoria::{commits_citados, report_tem_prova};

const IO_PADRAO_S: u64 = 8;
const IO_MIN_S: f64 = 1.0;
const IO_MAX_S: f64 = 60.0;
/// Bytes lidos de report, brief, perguntas e plano: o resto não é conferido.
pub(crate) const LEITURA_MAX: u64 = 1 << 20;
const OUT_BUFFER: usize = 64 * 1024 * 1024;
const ESTADOS_FINAIS: [&str; 2] = ["done", "archived"];

/// O `\w` Unicode do Python, como o TS o escreve (`src/lib/cerberus-vigilia.ts:21`).
pub(crate) const W: &str = r"\p{L}\p{N}_";

fn nivel(r: Resultado) -> u8 {
    match r {
        Resultado::Ok => 0,
        Resultado::Aviso => 1,
        _ => 2,
    }
}

/// `Indisponivel` (`src/lib/cerberus-vigilia.ts:29`): não deu para conferir; vira aviso com a evidência do que faltou.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indisponivel {
    pub conferido: String,
    pub onde: String,
}

impl Indisponivel {
    pub fn novo(conferido: impl Into<String>, onde: impl Into<String>) -> Self {
        Self {
            conferido: conferido.into(),
            onde: onde.into(),
        }
    }
}

/// Um achado da guarda (`src/lib/cerberus-vigilia.ts:40`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Achado {
    pub nivel: Resultado,
    pub conferido: String,
    pub onde: String,
}

/// `Achados` (`src/lib/cerberus-vigilia.ts:42`).
#[derive(Clone, Debug, Default)]
pub struct Achados {
    pub itens: Vec<Achado>,
}

impl Achados {
    pub fn add(&mut self, nivel: Resultado, conferido: impl Into<String>, onde: impl Into<String>) {
        self.itens.push(Achado {
            nivel,
            conferido: conferido.into(),
            onde: onde.into(),
        });
    }

    pub fn falta(&mut self, e: Indisponivel) {
        self.add(
            Resultado::Aviso,
            format!("não conferido: {}", e.conferido),
            e.onde,
        );
    }

    /// O pior nível; sem achados, `ok`.
    pub fn resultado(&self) -> Resultado {
        self.itens
            .iter()
            .map(|i| i.nivel)
            .fold(
                Resultado::Ok,
                |pior, n| if nivel(n) > nivel(pior) { n } else { pior },
            )
    }
}

fn tamanho_js(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// `curto` (`src/lib/cerberus-vigilia.ts:55`): espaços colapsados e corte em `n` unidades UTF-16, com reticências.
pub fn curto(s: &str, n: usize) -> String {
    let text = s
        .split(js_space)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if tamanho_js(&text) <= n {
        text
    } else {
        format!("{}…", js_slice(&text, n.saturating_sub(1)))
    }
}

/// `emitir` (`src/lib/cerberus-vigilia.ts:61`): os itens piores primeiro, até o limite de evidência do corpo.
pub fn emitir(achados: &Achados, detalhe_ok: &str) -> CheckVerdict {
    let res = achados.resultado();
    let mut itens: Vec<&Achado> = achados.itens.iter().collect();
    itens.sort_by_key(|i| std::cmp::Reverse(nivel(i.nivel)));
    let mut evidencia: Vec<Evid> = itens
        .iter()
        .map(|i| Evid {
            conferido: curto(
                &if i.nivel == Resultado::Ok {
                    i.conferido.clone()
                } else {
                    format!("[{}] {}", i.nivel.as_str(), i.conferido)
                },
                TEXT_MAX,
            ),
            onde: curto(&i.onde, TEXT_MAX),
        })
        .collect();
    if evidencia.len() > EVID_MAX {
        let resto = evidencia.len() - EVID_MAX + 1;
        evidencia.truncate(EVID_MAX - 1);
        evidencia.push(Evid {
            conferido: format!("mais {resto} itens conferidos, omitidos"),
            onde: "limite de evidência do corpo".into(),
        });
    }
    if evidencia.is_empty() {
        evidencia.push(Evid {
            conferido: "nada a conferir".into(),
            onde: "contexto da guarda".into(),
        });
    }
    let detalhe = if res == Resultado::Ok {
        detalhe_ok.to_owned()
    } else {
        itens
            .iter()
            .find(|i| i.nivel != Resultado::Ok)
            .map(|i| i.conferido.clone())
            .unwrap_or_default()
    };
    CheckVerdict {
        resultado: res,
        evidencia,
        detalhe: Some(curto(&detalhe, DETAIL_MAX)),
    }
}

// ---------------------------------------------------------------- contexto e borda: comandos e arquivos

/// `Vigilia` (`src/lib/cerberus-vigilia.ts:78`): o contexto injetado, mais a configuração do Cerberus lida uma vez.
pub struct Vigilia<'a> {
    pub ce: &'a CheckEnv,
    cached: OnceCell<Cfg>,
}

impl<'a> Vigilia<'a> {
    pub fn novo(ce: &'a CheckEnv) -> Self {
        Self {
            ce,
            cached: OnceCell::new(),
        }
    }

    /// `cfg` (`src/lib/cerberus-vigilia.ts:85`). O `config` do Rust não falha, então o `{}` do TS não ocorre.
    pub fn cfg(&self) -> &Cfg {
        self.cached.get_or_init(|| {
            let skill = if self.ce.skill.is_empty() {
                skill_root(&self.ce.hades_env())
            } else {
                self.ce.skill.clone().into()
            };
            let op = Opcoes::novo(self.ce.hades_env(), self.ce.dir.clone()).com_skill(skill);
            config(&op, true)
        })
    }

    /// `ioS` (`src/lib/cerberus-vigilia.ts:96`): inteiro de 1 a 60, senão o padrão (8).
    pub fn io_s(&self) -> u64 {
        let Some(bruto) = self.cfg().get("HADES_CERBERUS_VIGILIA_IO_S") else {
            return IO_PADRAO_S;
        };
        match js_number(bruto) {
            Some(n) if n.fract() == 0.0 && (IO_MIN_S..=IO_MAX_S).contains(&n) => n as u64,
            _ => IO_PADRAO_S,
        }
    }

    /// `reportsDir` (`src/lib/cerberus-vigilia.ts:101`).
    pub fn reports_dir(&self) -> String {
        match self.cfg().get("HADES_ORCH_REPORTS_DIR") {
            Some(d) if !d.is_empty() => d.clone(),
            _ => ".design-sync-reports".into(),
        }
    }

    /// `work` (`src/lib/cerberus-vigilia.ts:105`).
    pub fn work(&self) -> String {
        work_root(&self.ce.hades_env())
            .to_string_lossy()
            .into_owned()
    }

    fn home(&self) -> String {
        self.ce
            .var("HOME")
            .map(str::to_owned)
            .or_else(|| std::env::var("HOME").ok())
            .unwrap_or_default()
    }

    /// `expandUser` (`src/lib/cerberus-vigilia.ts:183`).
    pub(crate) fn expand_user(&self, p: &str) -> String {
        if p == "~" || p.starts_with("~/") {
            js_join(&[&self.home(), &p[1..]])
        } else {
            p.to_owned()
        }
    }

    /// `rodar` (`src/lib/cerberus-vigilia.ts:110`): `(exit, stdout)`; comando ausente ou tempo esgotado é
    /// [`Indisponivel`]. Divergência: morte por sinal sai 1 (o `proc::run` do core), onde o TS dá -1.
    pub fn rodar(&self, argv: &[&str], cwd: Option<&str>) -> Result<(i32, String), Indisponivel> {
        let limite = self.io_s();
        let cmd = argv.first().copied().unwrap_or("");
        let mut onde = js_slice(&argv.join(" "), 200);
        if let Some(c) = cwd {
            onde.push_str(&format!(" em {c}"));
        }
        let achou = if cmd.contains('/') {
            Path::new(cwd.unwrap_or(".")).join(cmd).exists()
        } else {
            which(cmd, self.ce.var("PATH").unwrap_or("")).is_some()
        };
        if !achou {
            return Err(Indisponivel::novo(
                format!("{cmd} não encontrado no PATH"),
                onde,
            ));
        }
        let env = self
            .ce
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let r = run(
            cmd,
            &argv[1..],
            &RunOptions {
                cwd: cwd.map(Into::into),
                timeout: Some(Duration::from_secs(limite)),
                max_buffer: Some(OUT_BUFFER),
                env,
                stdin: None,
            },
        );
        if r.timed_out {
            return Err(Indisponivel::novo(
                format!("{cmd} passou de {limite} s"),
                onde,
            ));
        }
        Ok((r.status, r.stdout))
    }

    /// `git` (`src/lib/cerberus-vigilia.ts:121`).
    pub fn git(&self, wt: &str, args: &[&str]) -> Result<(i32, String), Indisponivel> {
        let mut argv = vec!["git", "-C", wt];
        argv.extend_from_slice(args);
        self.rodar(&argv, None)
    }
}

/// `Number(texto)` do JS para o que a configuração guarda: aparado, vazio é 0, aceita `0x`/`0o`/`0b`.
fn js_number(texto: &str) -> Option<f64> {
    let t = js_trim(texto);
    if t.is_empty() {
        return Some(0.0);
    }
    for (prefixo, base) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(resto) = t.strip_prefix(prefixo) {
            return i64::from_str_radix(resto, base).ok().map(|n| n as f64);
        }
    }
    if t.chars()
        .any(|c| c.is_ascii_alphabetic() && !matches!(c, 'e' | 'E'))
        && !matches!(t.trim_start_matches(['+', '-']), "Infinity")
    {
        return None;
    }
    t.parse::<f64>().ok()
}

/// O texto do erro de E/S como o `ERRNO_TEXT` (`src/lib/cerberus-vigilia.ts:126`) o mostra; os outros códigos saem
/// com o texto do sistema (o TS mostra o nome do código).
pub(crate) fn errno_texto(e: &io::Error) -> String {
    match e.raw_os_error() {
        Some(21) => "Is a directory".into(),
        Some(13) => "Permission denied".into(),
        Some(20) => "Not a directory".into(),
        _ => e.to_string(),
    }
}

/// `ler` (`src/lib/cerberus-vigilia.ts:129`): texto do arquivo (até o limite), `None` se não existe.
pub fn ler(path: &str, limite: u64) -> Result<Option<String>, Indisponivel> {
    let falha =
        |e: io::Error| Indisponivel::novo(format!("leitura falhou ({})", errno_texto(&e)), path);
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(falha(e)),
    };
    let mut buf = Vec::new();
    f.take(limite).read_to_end(&mut buf).map_err(falha)?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// `lerCauda` (`src/lib/cerberus-vigilia.ts:150`): os últimos `limite` bytes.
pub(crate) fn ler_cauda(path: &str, limite: u64) -> io::Result<String> {
    let mut f = File::open(path)?;
    let size = f.metadata()?.len();
    f.seek(SeekFrom::Start(size.saturating_sub(limite)))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

pub(crate) fn is_dir(p: &str) -> bool {
    Path::new(p).is_dir()
}

pub(crate) fn is_file(p: &str) -> bool {
    Path::new(p).is_file()
}

// ---------------------------------------------------------------- caminhos como o `node:path` (POSIX)

/// `path.normalize` do Node.
pub(crate) fn js_normalize(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let abs = p.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|s| *s != "..") {
                    out.pop();
                } else if !abs {
                    out.push("..");
                }
            }
            s => out.push(s),
        }
    }
    let mut s = out.join("/");
    if s.is_empty() && !abs {
        s.push('.');
    }
    if !s.is_empty() && p.ends_with('/') {
        s.push('/');
    }
    if abs { format!("/{s}") } else { s }
}

/// `path.join` do Node: concatena (um absoluto no meio não reinicia, ao contrário do `PathBuf::join`) e normaliza.
pub(crate) fn js_join(parts: &[&str]) -> String {
    let juntos: Vec<&str> = parts.iter().copied().filter(|p| !p.is_empty()).collect();
    if juntos.is_empty() {
        return ".".into();
    }
    js_normalize(&juntos.join("/"))
}

/// `path.dirname` do Node.
pub(crate) fn js_dirname(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return "/".into();
    }
    match t.rfind('/') {
        None => ".".into(),
        Some(i) => {
            let d = t[..i].trim_end_matches('/');
            if d.is_empty() { "/".into() } else { d.into() }
        }
    }
}

/// `path.basename` do Node, com o sufixo opcional.
pub(crate) fn js_basename(p: &str, ext: &str) -> String {
    let t = p.trim_end_matches('/');
    let nome = t.rsplit('/').next().unwrap_or("");
    match nome.strip_suffix(ext) {
        Some(sem) if !ext.is_empty() && !sem.is_empty() => sem.into(),
        _ => nome.into(),
    }
}

/// `path.resolve` do Node, a partir da pasta do processo.
pub(crate) fn js_resolve(p: &str) -> String {
    let base = if p.starts_with('/') {
        p.to_owned()
    } else {
        let cwd = std::env::current_dir().unwrap_or_default();
        format!("{}/{p}", cwd.to_string_lossy())
    };
    let n = js_normalize(&base);
    if n.len() > 1 {
        n.trim_end_matches('/').into()
    } else {
        n
    }
}

/// `real` (`src/lib/cerberus-vigilia.ts:184`): `realpath`, ou o caminho resolvido quando ele não existe.
pub(crate) fn real(p: &str) -> String {
    match fs::canonicalize(p) {
        Ok(r) if !p.is_empty() => r.to_string_lossy().into_owned(),
        _ => js_resolve(p),
    }
}

/// Nomes da pasta fora os ocultos, em ordem: o `*` do `globSync`.
fn entradas(dir: &str, ocultos: bool) -> Vec<String> {
    let mut nomes: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| ocultos || !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    nomes.sort();
    nomes
}

/// `<work>/*/<sub>` que existe, em ordem (o `globSync(...).sort(byParts)` do TS).
pub(crate) fn em_cada_repo(root: &str, sub: &str) -> Vec<String> {
    if !is_dir(root) {
        return Vec::new();
    }
    entradas(root, false)
        .into_iter()
        .map(|n| js_join(&[root, &n, sub]))
        .filter(|p| Path::new(p).exists())
        .collect()
}

/// `jsonsIn` (`src/lib/cerberus-vigilia.ts:193`): `*.json` direto na pasta, em ordem; pasta ausente dá lista vazia.
pub(crate) fn jsons_in(dir: &str) -> Vec<String> {
    entradas(dir, true)
        .into_iter()
        .filter(|n| n.ends_with(".json"))
        .map(|n| js_join(&[dir, &n]))
        .filter(|p| is_file(p))
        .collect()
}

// ---------------------------------------------------------------- agente no registro do orquestrador

/// `orchDirs` (`src/lib/cerberus-vigilia.ts:204`): `<work>/<owner__repo>/orchestration` de todos os repositórios.
fn orch_dirs(v: &Vigilia) -> Vec<String> {
    em_cada_repo(&v.work(), "orchestration")
        .into_iter()
        .filter(|p| is_dir(p))
        .collect()
}

/// `slugDoDir` (`src/lib/cerberus-vigilia.ts:210`): `owner__repo` do `remote.origin.url` da pasta.
fn slug_do_dir(v: &Vigilia, d: &str) -> Option<String> {
    let (rc, out) = v.git(d, &["config", "--get", "remote.origin.url"]).ok()?;
    let url = js_trim(&out);
    if rc != 0 || url.is_empty() {
        return None;
    }
    let url = url.strip_suffix(".git").unwrap_or(url);
    let parts: Vec<&str> = url.split(['/', ':']).collect();
    (parts.len() >= 2).then(|| parts[parts.len() - 2..].join("__"))
}

/// `Agente` (`src/lib/cerberus-vigilia.ts:224`): um registro de `<orch>/agents/<nome>.json`.
#[derive(Clone, Debug)]
pub struct Agente {
    pub nome: String,
    pub reg: Map<String, Value>,
    pub arquivo: String,
}

/// `text` (`src/lib/cerberus-vigilia.ts:247`): valor falso do JS vira vazio; o resto, `String(valor)`.
pub(crate) fn texto(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => String::new(),
        Some(Value::Number(n)) if n.as_f64().is_some_and(|f| f == 0.0) => String::new(),
        other => js_string(other),
    }
}

impl Agente {
    pub fn orch(&self) -> String {
        js_dirname(&js_dirname(&self.arquivo))
    }

    pub fn worktree(&self) -> String {
        texto(self.reg.get("worktree"))
    }

    pub fn modo(&self) -> String {
        texto(self.reg.get("mode"))
    }

    pub fn ativo(&self) -> bool {
        !ESTADOS_FINAIS.contains(&js_string(self.reg.get("state")).as_str())
    }

    pub(crate) fn campo(&self, chave: &str) -> String {
        texto(self.reg.get(chave))
    }
}

/// `carregar` (`src/lib/cerberus-vigilia.ts:249`). Divergência: o texto do erro do parser é o do `serde_json`.
fn carregar(file: &str) -> Result<Agente, Indisponivel> {
    let reg: Value = fs::read_to_string(file)
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
        .map_err(|m| Indisponivel::novo(format!("registro ilegível ({m})"), file))?;
    let Value::Object(dict) = js_norm(reg) else {
        return Err(Indisponivel::novo("registro não é um objeto JSON", file));
    };
    let nome = texto(dict.get("name"));
    Ok(Agente {
        nome: if nome.is_empty() {
            js_basename(file, ".json")
        } else {
            nome
        },
        reg: dict,
        arquivo: file.to_owned(),
    })
}

static NOME_AGENTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$").expect("regex"));

/// `pyQuote` (`src/lib/cerberus-vigilia.ts:274`): o `repr` de texto do Python.
pub(crate) fn py_quote(value: &str) -> String {
    if value.contains('\'') && !value.contains('"') {
        format!("\"{value}\"")
    } else {
        format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
    }
}

/// `agentePorNome` (`src/lib/cerberus-vigilia.ts:261`).
fn agente_por_nome(v: &Vigilia, nome: &str, d: &str) -> Result<Agente, Indisponivel> {
    if !NOME_AGENTE.is_match(nome) {
        return Err(Indisponivel::novo(
            format!("agente fora do padrão no contexto: {}", py_quote(nome)),
            "CERBERUS_CTX.agente",
        ));
    }
    let mut achados: Vec<String> = orch_dirs(v)
        .iter()
        .map(|o| js_join(&[o, "agents", &format!("{nome}.json")]))
        .filter(|f| is_file(f))
        .collect();
    if achados.len() > 1 {
        let slug = slug_do_dir(v, d);
        let do_repo: Vec<String> = achados
            .iter()
            .filter(|f| Some(js_basename(&js_dirname(&js_dirname(&js_dirname(f))), "")) == slug)
            .cloned()
            .collect();
        if !do_repo.is_empty() {
            achados = do_repo;
        }
    }
    match achados.as_slice() {
        [] => Err(Indisponivel::novo(
            format!("agente {nome} sem registro em <work>/*/orchestration/agents"),
            v.work(),
        )),
        [um] => carregar(um),
        varios => Err(Indisponivel::novo(
            format!("agente {nome} registrado em mais de um repositório"),
            varios.join(", "),
        )),
    }
}

/// `orchDoDir` (`src/lib/cerberus-vigilia.ts:278`).
fn orch_do_dir(v: &Vigilia, d: &str) -> Result<String, Indisponivel> {
    let o = slug_do_dir(v, d).map(|s| js_join(&[&v.work(), &s, "orchestration"]));
    match o {
        Some(o) if is_dir(&o) => Ok(o),
        o => Err(Indisponivel::novo(
            format!("sem registro do orquestrador para o repositório de {d}"),
            o.unwrap_or_else(|| d.to_owned()),
        )),
    }
}

/// `members` da consolidação, com os erros que o `(doc.members || []).map(String)` do TS levanta.
fn membros_de(doc: &Value) -> Result<Vec<String>, String> {
    let members = match doc {
        Value::Null => return Err("Cannot read properties of null (reading 'members')".into()),
        Value::Object(m) => m.get("members"),
        _ => None,
    };
    match members {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(Vec::new()),
        Some(Value::String(s)) if s.is_empty() => Ok(Vec::new()),
        Some(Value::Number(n)) if n.as_f64() == Some(0.0) => Ok(Vec::new()),
        Some(Value::Array(itens)) => Ok(itens.iter().map(|i| js_string(Some(i))).collect()),
        Some(_) => Err("(doc.members || []).map is not a function".into()),
    }
}

/// `alvos` (`src/lib/cerberus-vigilia.ts:286`): o agente do DONE, ou os membros da consolidação, e como foram
/// escolhidos.
pub fn alvos(v: &Vigilia) -> Result<(Vec<Agente>, String), Indisponivel> {
    let d = v.ce.dir.to_string_lossy().into_owned();
    if v.ce.momento == "done" || v.ce.ctx("agente").is_some() {
        let Some(agente) = v.ce.ctx("agente") else {
            return Err(Indisponivel::novo("contexto sem agente", "CERBERUS_CTX"));
        };
        let a = agente_por_nome(v, agente, &d)?;
        return Ok((vec![a], format!("agente {agente} (contexto do DONE)")));
    }
    let o = orch_do_dir(v, &d)?;
    let todos = jsons_in(&js_join(&[&o, "agents"]))
        .iter()
        .map(|f| carregar(f))
        .collect::<Result<Vec<_>, _>>()?;
    let br = v.ce.ctx("branch").unwrap_or("");
    let cons = js_join(&[
        &o,
        "consolidations",
        &format!("{}.json", br.replace('/', "_")),
    ]);
    let mut membros: Vec<String> = Vec::new();
    if !br.is_empty() && is_file(&cons) {
        let lido = fs::read_to_string(&cons)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
            .and_then(|doc| membros_de(&js_norm(doc)));
        match lido {
            Ok(m) => {
                for x in m {
                    if !membros.contains(&x) {
                        membros.push(x);
                    }
                }
            }
            Err(m) => {
                return Err(Indisponivel::novo(
                    format!("registro da consolidação ilegível ({m})"),
                    cons,
                ));
            }
        }
    }
    if !membros.is_empty() {
        let sel: Vec<Agente> = todos
            .into_iter()
            .filter(|a| membros.contains(&a.campo("pr")))
            .collect();
        let ordenados: BTreeSet<&String> = membros.iter().collect();
        let prs: Vec<&str> = ordenados.into_iter().map(String::as_str).collect();
        let como = format!(
            "membros da consolidação {br} (PRs {}): {} agentes no registro",
            prs.join(", "),
            sel.len()
        );
        return Ok((sel, como));
    }
    let sel: Vec<Agente> = todos
        .into_iter()
        .filter(|a| a.reg.get("state") != Some(&Value::String("archived".into())))
        .collect();
    let alvo = if br.is_empty() {
        "a consolidação"
    } else {
        br
    };
    let como = format!(
        "sem membros registrados para {alvo}: todos os {} agentes não arquivados de {o}",
        sel.len()
    );
    Ok((sel, como))
}

/// `reportPath` (`src/lib/cerberus-vigilia.ts:314`).
pub fn report_path(v: &Vigilia, a: &Agente) -> String {
    js_join(&[&a.worktree(), &v.reports_dir(), "report.md"])
}

/// `reportDone` (`src/lib/cerberus-vigilia.ts:316`): a primeira linha começa com `STATUS: DONE`.
pub fn report_done(texto: Option<&str>) -> bool {
    texto.is_some_and(|t| {
        !t.is_empty()
            && split_lines(t)
                .first()
                .is_some_and(|l| js_trim(l).starts_with("STATUS: DONE"))
    })
}

static ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[-*+]|[0-9]+[.)])\s").expect("regex"));

/// `blocos` (`src/lib/cerberus-vigilia.ts:322`): parágrafo, ou item de lista de primeiro nível com continuação e
/// subitens; cercas de código saem (o que está em ``` é saída citada, não afirmação). Cada bloco: (linha inicial,
/// linhas).
pub fn blocos(texto: &str) -> Vec<(usize, Vec<String>)> {
    let mut out = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut ini = 0;
    let mut cerca = false;
    for (idx, ln) in split_lines(texto).into_iter().enumerate() {
        let i = idx + 1;
        if ln.trim_start_matches(js_space).starts_with("```") {
            cerca = !cerca;
            continue;
        }
        if cerca {
            continue;
        }
        let vazia = js_trim(&ln).is_empty();
        let novo = vazia || ln.starts_with('#') || ITEM.is_match(&ln) || ln.starts_with('|');
        if novo && !cur.is_empty() {
            out.push((ini, std::mem::take(&mut cur)));
        }
        if !vazia {
            if cur.is_empty() {
                ini = i;
            }
            let fecha = ln.starts_with('#') || ln.starts_with('|');
            cur.push(ln);
            if fecha {
                out.push((ini, std::mem::take(&mut cur)));
            }
        }
    }
    if !cur.is_empty() {
        out.push((ini, cur));
    }
    out
}

/// `alvosOuFalha` (`src/lib/cerberus-vigilia.ts:352`).
pub(crate) fn alvos_ou_falha(v: &Vigilia, achados: &mut Achados) -> Vec<Agente> {
    match alvos(v) {
        Ok((lista, como)) => {
            achados.add(Resultado::Ok, como, "contexto do Cerberus");
            lista
        }
        Err(e) => {
            achados.falta(e);
            Vec::new()
        }
    }
}

/// `tentar` (`src/lib/cerberus-vigilia.ts:367`): [`Indisponivel`] vira achado `aviso`; devolve falso quando faltou.
pub(crate) fn tentar(achados: &mut Achados, r: Result<(), Indisponivel>) -> bool {
    match r {
        Ok(()) => true,
        Err(e) => {
            achados.falta(e);
            false
        }
    }
}

/// `reportDe` (`src/lib/cerberus-vigilia.ts:380`): o report, ou `None` (ausente vira `barra`).
pub(crate) fn report_de(v: &Vigilia, a: &Agente, achados: &mut Achados) -> Option<String> {
    let p = report_path(v, a);
    match ler(&p, LEITURA_MAX) {
        Err(e) => {
            achados.falta(e);
            None
        }
        Ok(None) => {
            achados.add(
                Resultado::Barra,
                format!("{}: report.md ausente", a.nome),
                p,
            );
            None
        }
        Ok(t) => t,
    }
}

/// Assinatura de uma guarda da Vigília (`Guard`, `src/lib/cerberus-vigilia.ts:390`).
pub type Guarda = fn(&Vigilia, &mut Achados);

/// `Entrada` (`src/lib/cerberus-vigilia.ts:1085`): a guarda e o detalhe do veredito `ok`.
#[derive(Clone, Copy)]
pub struct Entrada {
    pub guarda: Guarda,
    pub ok: &'static str,
}

/// `AUDITORIA` (`src/lib/cerberus-vigilia.ts:1087`), na ordem do TS.
pub const AUDITORIA: [(&str, Entrada); 6] = [
    (
        "status",
        Entrada {
            guarda: auditoria::status,
            ok: "report em STATUS: DONE",
        },
    ),
    (
        "gates",
        Entrada {
            guarda: auditoria::gates,
            ok: "gates citados têm registro rc=0",
        },
    ),
    (
        "capturas",
        Entrada {
            guarda: auditoria::capturas,
            ok: "capturas citadas existem",
        },
    ),
    (
        "commits",
        Entrada {
            guarda: auditoria::commits,
            ok: "commits citados estão na branch",
        },
    ),
    (
        "escopo",
        Entrada {
            guarda: auditoria::escopo,
            ok: "arquivos tocados cabem no escopo do brief",
        },
    ),
    (
        "perguntas",
        Entrada {
            guarda: auditoria::perguntas,
            ok: "nenhuma pergunta aberta com o report em DONE",
        },
    ),
];

/// `COERENCIA` (`src/lib/cerberus-vigilia.ts:1096`), na ordem do TS.
pub const COERENCIA: [(&str, Entrada); 2] = [
    (
        "registro",
        Entrada {
            guarda: coerencia::registro,
            ok: "registro confere com git, herdr e GitHub",
        },
    ),
    (
        "plano",
        Entrada {
            guarda: coerencia::plano,
            ok: "Estado atual do plano confere com o registro",
        },
    ),
];

/// O uso que `auditoria.py` imprimia para guarda desconhecida (`usageOf`, `src/commands/cerberus-check.ts:24`).
pub fn uso_auditoria() -> String {
    let nomes: Vec<&str> = AUDITORIA.iter().map(|(n, _)| *n).collect();
    format!("uso: auditoria.py {}", nomes.join("|"))
}

/// O uso que `coerencia.py` imprimia para guarda desconhecida (`usageOf`, `src/commands/cerberus-check.ts:28`).
pub fn uso_coerencia() -> String {
    let nomes: Vec<&str> = COERENCIA.iter().map(|(n, _)| *n).collect();
    format!("uso: coerencia.py {}", nomes.join("|"))
}

/// `rodarVigilia` (`src/lib/cerberus-vigilia.ts:1101`): `None` para guarda fora da tabela.
pub fn rodar_vigilia(
    tabela: &[(&str, Entrada)],
    nome: &str,
    ce: &CheckEnv,
) -> Option<CheckVerdict> {
    let (_, entrada) = tabela.iter().find(|(n, _)| *n == nome)?;
    let mut achados = Achados::default();
    (entrada.guarda)(&Vigilia::novo(ce), &mut achados);
    Some(emitir(&achados, entrada.ok))
}

#[cfg(test)]
mod tests;
