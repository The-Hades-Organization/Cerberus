//! Base dos checks portados (`src/lib/cerberus-check.ts`): contexto injetado, comandos com limite de tempo e o
//! veredito. Cada check é uma função pura do ambiente que devolve o veredito; quem imprime é a borda (o subcomando
//! `cerberus check`). O contrato de saída é o de sempre: uma linha JSON `{resultado, evidencia, detalhe?}`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hades_core::Env;
use hades_core::proc::{RunOptions, run, which};
use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::modelo::{Resultado, js_string, js_trim};

/// `TIMEOUT_S` (`src/lib/cerberus-check.ts:8`).
pub const TIMEOUT_S: u64 = 20;
/// `EVID_MAX_CHARS` (`src/lib/cerberus-check.ts:9`).
pub const EVID_MAX_CHARS: usize = 500;
const OUTPUT_BUFFER_MAX: usize = 256 * 1024 * 1024;
const EVID_MAX_ITEMS: usize = 20;
const DETAIL_MAX_CHARS: usize = 1000;

/// `Evid` (`src/lib/cerberus-check.ts:16`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evid {
    pub conferido: String,
    pub onde: String,
}

/// `CheckVerdict` (`src/lib/cerberus-check.ts:17`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckVerdict {
    pub resultado: Resultado,
    pub evidencia: Vec<Evid>,
    pub detalhe: Option<String>,
}

impl CheckVerdict {
    /// O objeto que a borda imprime, nesta ordem de chaves: `resultado`, `evidencia`, `detalhe`.
    pub fn to_json(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("resultado".into(), self.resultado.as_str().into());
        let evid = self
            .evidencia
            .iter()
            .map(|e| {
                let mut o = Map::new();
                o.insert("conferido".into(), e.conferido.clone().into());
                o.insert("onde".into(), e.onde.clone().into());
                Value::Object(o)
            })
            .collect();
        obj.insert("evidencia".into(), Value::Array(evid));
        if let Some(d) = &self.detalhe {
            obj.insert("detalhe".into(), d.clone().into());
        }
        Value::Object(obj)
    }
}

/// `CheckEnv` (`src/lib/cerberus-check.ts:20`): o que o núcleo passa a um check exec (variáveis `CERBERUS_*`),
/// injetado em vez de lido do ambiente global. `env` é o ambiente inteiro que os subprocessos recebem.
#[derive(Clone, Debug, Default)]
pub struct CheckEnv {
    pub dir: PathBuf,
    pub skill: String,
    pub momento: String,
    pub ctx: IndexMap<String, String>,
    pub env: BTreeMap<String, String>,
}

impl CheckEnv {
    /// `checkEnvFromProcess` (`src/lib/cerberus-check.ts:28`), sobre um ambiente dado.
    pub fn do_ambiente(env: BTreeMap<String, String>, cwd: &Path) -> Self {
        let get = |k: &str| env.get(k).cloned();
        let dir = get("CERBERUS_DIR")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.to_path_buf());
        Self {
            dir,
            skill: get("CERBERUS_SKILL").unwrap_or_default(),
            momento: get("CERBERUS_MOMENTO").unwrap_or_default(),
            ctx: parse_ctx(get("CERBERUS_CTX").as_deref()),
            env,
        }
    }

    /// `ce.ctx.<chave>`, vazio contando como ausente (o `||`/truthy do TS).
    pub fn ctx(&self, key: &str) -> Option<&str> {
        self.ctx
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// `ce.env.<chave>` cru (`??` do TS: só a ausente cai).
    pub fn var(&self, key: &str) -> Option<&str> {
        self.env.get(key).map(String::as_str)
    }

    /// O mesmo ambiente no tipo do `hades-core`.
    pub fn hades_env(&self) -> Env {
        self.env
            .iter()
            .fold(Env::empty(), |e, (k, v)| e.with(k, v.clone()))
    }
}

/// `parseCtx` (`src/lib/cerberus-check.ts:33`): objeto JSON vira `chave → String(valor)`; o resto, vazio.
fn parse_ctx(text: Option<&str>) -> IndexMap<String, String> {
    let text = text.filter(|t| !t.is_empty()).unwrap_or("{}");
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| (k.clone(), js_string(Some(v))))
            .collect(),
        _ => IndexMap::new(),
    }
}

/// `.slice(0, max)` do JS: conta unidades UTF-16. Um caractere fora do BMP cortado ao meio sai inteiro de fora
/// (o JS deixaria meio par substituto, que não existe em `String`).
pub fn js_slice(text: &str, max: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in text.chars() {
        used += c.len_utf16();
        if used > max {
            break;
        }
        out.push(c);
    }
    out
}

/// `ev` (`src/lib/cerberus-check.ts:43`).
pub fn ev(conferido: impl AsRef<str>, onde: impl AsRef<str>) -> Evid {
    Evid {
        conferido: js_slice(conferido.as_ref(), EVID_MAX_CHARS),
        onde: js_slice(onde.as_ref(), EVID_MAX_CHARS),
    }
}

/// `verdict` (`src/lib/cerberus-check.ts:47`): sem evidência entra a de guarda vazia; detalhe vazio some.
pub fn verdict(resultado: Resultado, evidencia: Vec<Evid>, detalhe: Option<&str>) -> CheckVerdict {
    let evidencia = if evidencia.is_empty() {
        vec![Evid {
            conferido: "nada conferido".into(),
            onde: "guarda sem evidência".into(),
        }]
    } else {
        evidencia.into_iter().take(EVID_MAX_ITEMS).collect()
    };
    CheckVerdict {
        resultado,
        evidencia,
        detalhe: detalhe
            .filter(|d| !d.is_empty())
            .map(|d| js_slice(d, DETAIL_MAX_CHARS)),
    }
}

/// `Ran` (`src/lib/cerberus-check.ts:53`): `code` ausente quando o binário não existe ou o tempo estourou.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Opções de [`exec`]: pasta (padrão `ce.dir`), variáveis extras e limite de tempo (padrão [`TIMEOUT_S`]).
#[derive(Clone, Debug, Default)]
pub struct ExecOpts {
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub timeout_s: Option<u64>,
}

/// O executável existe? Com `/` é caminho; sem, procura no `PATH` do ambiente do check (o ENOENT do `spawnSync`).
fn encontra(cmd: &str, ce: &CheckEnv, cwd: &Path) -> bool {
    if cmd.contains('/') {
        return cwd.join(cmd).exists();
    }
    which(cmd, ce.var("PATH").unwrap_or("")).is_some()
}

/// `exec` (`src/lib/cerberus-check.ts:56`). O filho recebe o ambiente do check mais `opts.env`. Divergência: falha de
/// spawn que não é binário ausente (EACCES e afins) sai como `code` 1 com stderr vazio, o que o `proc::run` devolve.
pub fn exec(argv: &[&str], ce: &CheckEnv, opts: &ExecOpts) -> Ran {
    let timeout_s = opts.timeout_s.unwrap_or(TIMEOUT_S);
    let cmd = argv.first().copied().unwrap_or("");
    let cwd = opts.cwd.clone().unwrap_or_else(|| ce.dir.clone());
    if !encontra(cmd, ce, &cwd) {
        return Ran {
            code: None,
            stdout: String::new(),
            stderr: format!("{cmd}: não encontrado no PATH"),
        };
    }
    let mut env: Vec<(String, String)> =
        ce.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    env.extend(opts.env.iter().cloned());
    let r = run(
        cmd,
        &argv[1..],
        &RunOptions {
            cwd: Some(cwd),
            timeout: Some(Duration::from_secs(timeout_s)),
            max_buffer: Some(OUTPUT_BUFFER_MAX),
            env,
            stdin: None,
        },
    );
    if r.timed_out {
        let head: Vec<&str> = argv.iter().take(3).copied().collect();
        return Ran {
            code: None,
            stdout: String::new(),
            stderr: format!("{}: passou de {timeout_s} s", head.join(" ")),
        };
    }
    Ran {
        code: Some(r.status),
        stdout: r.stdout,
        stderr: js_trim(&r.stderr).to_owned(),
    }
}

/// `git` (`src/lib/cerberus-check.ts:69`).
pub fn git(ce: &CheckEnv, args: &[&str]) -> Ran {
    let mut argv = vec!["git"];
    argv.extend_from_slice(args);
    exec(&argv, ce, &ExecOpts::default())
}

/// `profileField` (`src/lib/cerberus-check.ts:74`): campo do perfil ativo do diretório, aparado; `None` sem perfil,
/// sem o campo ou vazio.
pub fn profile_field(ce: &CheckEnv, field: &str) -> Option<String> {
    let env = ce.hades_env();
    let resolution = hades_core::project::resolve_project(&env, &ce.dir);
    let profile = hades_core::profile::load_profile(&resolution);
    let value = hades_core::profile::get_field(&profile, field, None)?;
    let value = js_trim(&value);
    (!value.is_empty()).then(|| value.to_owned())
}

/// `/[:/]([^/:]+)\/([^/]+?)(?:\.git)?$/` do JS sobre a URL do remote: `(owner, repo)` do casamento mais à esquerda.
pub(crate) fn owner_repo(url: &str) -> Option<(String, String)> {
    url.char_indices()
        .filter(|(_, c)| matches!(c, ':' | '/'))
        .find_map(|(i, _)| {
            let rest = &url[i + 1..];
            let (owner, repo) = rest.split_once('/')?;
            if owner.is_empty() || owner.contains(':') || repo.is_empty() || repo.contains('/') {
                return None;
            }
            let repo = match repo.strip_suffix(".git") {
                Some(sem) if !sem.is_empty() => sem,
                _ => repo,
            };
            Some((owner.to_owned(), repo.to_owned()))
        })
}

/// `ghRepoSlug` (`src/lib/cerberus-check.ts:79`).
pub fn gh_repo_slug(ce: &CheckEnv) -> Option<String> {
    let r = git(ce, &["config", "--get", "remote.origin.url"]);
    if r.code != Some(0) {
        return None;
    }
    owner_repo(js_trim(&r.stdout)).map(|(o, r)| format!("{o}/{r}"))
}

/// `BranchInfo` (`src/lib/cerberus-check.ts:84`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchInfo {
    pub branch: String,
    pub remote: String,
    pub merge: String,
    pub push_remote: String,
}

/// `branchInfo` (`src/lib/cerberus-check.ts:87`): onde o push da branch atual vai, pela configuração do git.
pub fn branch_info(ce: &CheckEnv) -> BranchInfo {
    let head = git(ce, &["symbolic-ref", "--short", "-q", "HEAD"]);
    let branch = if head.code == Some(0) {
        js_trim(&head.stdout).to_owned()
    } else {
        String::new()
    };
    if branch.is_empty() {
        return BranchInfo::default();
    }
    let cfg = |key: &str| js_trim(&git(ce, &["config", "--get", key]).stdout).to_owned();
    let remote = cfg(&format!("branch.{branch}.remote"));
    let merge = cfg(&format!("branch.{branch}.merge"));
    let mut push_remote = cfg(&format!("branch.{branch}.pushRemote"));
    if push_remote.is_empty() {
        push_remote = cfg("remote.pushDefault");
    }
    if push_remote.is_empty() {
        push_remote = remote.clone();
    }
    let merge = merge
        .strip_prefix("refs/heads/")
        .map(str::to_owned)
        .unwrap_or(merge);
    BranchInfo {
        branch,
        remote,
        merge,
        push_remote,
    }
}

/// `HeadRef` (`src/lib/cerberus-check.ts:97`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeadRef {
    pub head: Option<String>,
    pub head_ev: Option<Evid>,
    pub error: Option<Evid>,
}

/// `headRef` (`src/lib/cerberus-check.ts:100`): `headRefName` da PR do contexto, pelo `head` do contexto ou por
/// `gh pr view <pr>`.
pub fn head_ref(ce: &CheckEnv) -> HeadRef {
    if let Some(head) = ce.ctx("head") {
        return HeadRef {
            head: Some(head.to_owned()),
            head_ev: Some(ev(
                format!("headRefName = {head}"),
                "contexto da avaliação (--ctx head=)",
            )),
            error: None,
        };
    }
    if let Some(pr) = ce.ctx("pr") {
        let r = exec(
            &[
                "gh",
                "pr",
                "view",
                pr,
                "--json",
                "headRefName",
                "--jq",
                ".headRefName",
            ],
            ce,
            &ExecOpts::default(),
        );
        let name = js_trim(&r.stdout);
        if r.code == Some(0) && !name.is_empty() {
            return HeadRef {
                head: Some(name.to_owned()),
                head_ev: Some(ev(
                    format!("headRefName da PR #{pr} = {name}"),
                    format!("gh pr view {pr}"),
                )),
                error: None,
            };
        }
        let motivo = if r.stderr.is_empty() {
            "resposta vazia"
        } else {
            &r.stderr
        };
        return HeadRef {
            error: Some(ev(
                format!("não consegui ler o headRefName da PR #{pr}: {motivo}"),
                format!("gh pr view {pr} --json headRefName"),
            )),
            ..HeadRef::default()
        };
    }
    HeadRef {
        error: Some(ev(
            "sem pr nem head no contexto",
            "CERBERUS_CTX (--ctx pr=<n> ou --ctx head=<branch>)",
        )),
        ..HeadRef::default()
    }
}

enum Peca {
    Qualquer,
    Um,
    Lit(char),
    Conjunto {
        nega: bool,
        itens: Vec<(char, char)>,
    },
}

/// O conjunto `[...]` como o TS o passa à classe do JS: `!` ou `^` no começo nega, `a-b` é faixa, `\` é literal.
fn conjunto(corpo: &[char]) -> Peca {
    let (nega, corpo) = match corpo.first() {
        Some('!' | '^') => (true, &corpo[1..]),
        _ => (false, corpo),
    };
    let mut itens = Vec::new();
    let mut i = 0;
    while i < corpo.len() {
        let a = corpo[i];
        if i + 2 < corpo.len() && corpo[i + 1] == '-' {
            itens.push((a, corpo[i + 2]));
            i += 3;
        } else {
            itens.push((a, a));
            i += 1;
        }
    }
    Peca::Conjunto { nega, itens }
}

fn pecas(pattern: &str) -> Vec<Peca> {
    let p: Vec<char> = pattern.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < p.len() {
        match p[i] {
            '*' => out.push(Peca::Qualquer),
            '?' => out.push(Peca::Um),
            '[' => match (i + 2..p.len()).find(|&j| p[j] == ']') {
                Some(close) => {
                    out.push(conjunto(&p[i + 1..close]));
                    i = close;
                }
                None => out.push(Peca::Lit('[')),
            },
            c => out.push(Peca::Lit(c)),
        }
        i += 1;
    }
    out
}

fn casa(pecas: &[Peca], texto: &[char]) -> bool {
    let Some((primeira, resto)) = pecas.split_first() else {
        return texto.is_empty();
    };
    match primeira {
        Peca::Qualquer => (0..=texto.len()).any(|k| casa(resto, &texto[k..])),
        outra => texto.split_first().is_some_and(|(c, t)| {
            let ok = match outra {
                Peca::Um => true,
                Peca::Lit(l) => l == c,
                Peca::Conjunto { nega, itens } => {
                    itens.iter().any(|&(a, b)| (a..=b).contains(c)) != *nega
                }
                Peca::Qualquer => unreachable!(),
            };
            ok && casa(resto, t)
        }),
    }
}

/// `fnmatchCase` (`src/lib/cerberus-check.ts:113`): `*`, `?` e `[seq]` sobre o texto inteiro, sem regex. Divergência:
/// faixa invertida (`[z-a]`) não casa nada, onde o TS lança erro de regex.
pub fn fnmatch_case(text: &str, pattern: &str) -> bool {
    let texto: Vec<char> = text.chars().collect();
    casa(&pecas(pattern), &texto)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnmatch_como_o_ts() {
        assert!(fnmatch_case("a.devtunnels.ms", "*.devtunnels.ms"));
        assert!(!fnmatch_case("evil.example.com", "*.devtunnels.ms"));
        assert!(fnmatch_case("a1.devtunnels.ms", "a?.devtunnels.ms"));
        assert!(fnmatch_case("b.x.io", "[ab].x.io"));
        assert!(!fnmatch_case("c.x.io", "[ab].x.io"));
        assert!(fnmatch_case("c.x.io", "[!ab].x.io"));
        assert!(fnmatch_case("c.x.io", "[^ab].x.io"));
        assert!(fnmatch_case("k", "[a-z]"));
        assert!(fnmatch_case("]", "[]]"));
        assert!(fnmatch_case("[a", "[a"));
        assert!(fnmatch_case("x", "[!]"));
        assert!(fnmatch_case("a.b", "a.b"));
        assert!(!fnmatch_case("axb", "a.b"));
        assert!(fnmatch_case("ab\ncd", "a*d"));
        assert!(!fnmatch_case("A", "a"));
    }

    #[test]
    fn owner_repo_como_a_regex() {
        let o = |u: &str| owner_repo(u).map(|(a, b)| format!("{a}/{b}"));
        assert_eq!(
            o("git@github.com:acme/demo.git").as_deref(),
            Some("acme/demo")
        );
        assert_eq!(
            o("https://github.com/acme/demo").as_deref(),
            Some("acme/demo")
        );
        assert_eq!(
            o("https://github.com/acme/demo.git").as_deref(),
            Some("acme/demo")
        );
        assert_eq!(o("/srv/git/acme/.git").as_deref(), Some("acme/.git"));
        assert_eq!(o("solto"), None);
        assert_eq!(o(""), None);
    }

    #[test]
    fn veredito_corta_e_preenche() {
        let v = verdict(Resultado::Ok, vec![], Some(""));
        assert_eq!(
            hades_core::jsonfmt::py_dumps(&v.to_json(), None),
            r#"{"resultado": "ok", "evidencia": [{"conferido": "nada conferido", "onde": "guarda sem evidência"}]}"#
        );
        let longa = "é".repeat(600);
        assert_eq!(ev(&longa, "x").conferido.chars().count(), 500);
        assert_eq!(js_slice("a😀b", 2), "a");
        let muitas = (0..30).map(|i| ev(i.to_string(), "x")).collect();
        assert_eq!(verdict(Resultado::Barra, muitas, None).evidencia.len(), 20);
    }

    #[test]
    fn ctx_do_ambiente() {
        let env: BTreeMap<String, String> = [
            (
                "CERBERUS_CTX",
                r#"{"pr": 1234, "head": "feat/x", "n": null}"#,
            ),
            ("CERBERUS_MOMENTO", "push"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let ce = CheckEnv::do_ambiente(env, Path::new("/x"));
        assert_eq!(ce.dir, PathBuf::from("/x"));
        assert_eq!(ce.ctx("pr"), Some("1234"));
        assert_eq!(ce.ctx.get("n").map(String::as_str), Some("null"));
        assert_eq!(ce.momento, "push");
        assert!(parse_ctx(Some("[1]")).is_empty());
        assert!(parse_ctx(Some("{x")).is_empty());
    }
}
