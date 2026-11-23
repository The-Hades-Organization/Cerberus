//! As 8 guardas da Porta (`src/lib/cerberus-porta.ts`). Cada guarda é uma função do ambiente injetado
//! ([`CheckEnv`]) que devolve o veredito, com o mesmo contrato de entrada e saída do check exec.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use hades_core::config::split_lines;
use hades_core::paths::{skill_root, work_root};
use hades_core::registry::plan_sha;
use regex::Regex;
use serde_json::Value;

use crate::Opcoes;
use crate::check::{
    CheckEnv, CheckVerdict, Evid, ExecOpts, branch_info, ev, exec, fnmatch_case, gh_repo_slug, git,
    head_ref, owner_repo, profile_field, verdict,
};
use crate::config::config;
use crate::modelo::{Resultado, js_space, js_string, js_trim};

/// Assinatura de uma guarda (`Guard`, `src/lib/cerberus-porta.ts:14`).
pub type Guarda = fn(&CheckEnv) -> CheckVerdict;

const TRONCOS: [&str; 4] = ["main", "master", "trunk", "develop"];
const NO_PUSH: &str = "no_push";
const GUARD_TIMEOUT_S: u64 = 90;
const MAX_SCANNED_FILES: usize = 2000;
const MAX_EVIDENCE: usize = 20;
const MASK_PREFIX: usize = 4;

use Resultado::{Aviso, Barra, Ok as Passa};

fn ou<'a>(texto: &'a str, padrao: &'a str) -> &'a str {
    if texto.is_empty() { padrao } else { texto }
}

/// A skill que o núcleo usou (`CERBERUS_SKILL`), ou a instalada. No TS é `skillRoot`, a pasta do código.
fn skill_de(ce: &CheckEnv) -> PathBuf {
    if ce.skill.is_empty() {
        skill_root(&ce.hades_env())
    } else {
        PathBuf::from(&ce.skill)
    }
}

fn texto_de(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------- branch-protegida

/// `branchProtegida` (`src/lib/cerberus-porta.ts:25`).
pub fn branch_protegida(ce: &CheckEnv) -> CheckVerdict {
    let bi = branch_info(ce);
    let dest = [
        bi.merge.as_str(),
        ce.ctx("head").unwrap_or(""),
        bi.branch.as_str(),
    ]
    .into_iter()
    .find(|d| !d.is_empty())
    .unwrap_or("")
    .to_owned();
    if dest.is_empty() {
        return verdict(
            Barra,
            vec![ev(
                "HEAD destacado e sem head no contexto: destino desconhecido",
                "git symbolic-ref HEAD",
            )],
            None,
        );
    }
    let mut evid = vec![ev(
        format!("destino do push = {dest}"),
        "upstream da branch (git config branch.<b>.merge), ou head do contexto",
    )];
    match profile_field(ce, "flow.base") {
        Some(base) => {
            evid.push(ev(
                format!("base do projeto (flow.base) = {base}"),
                "perfil ativo: hades project get flow.base",
            ));
            if dest == base {
                return verdict(Barra, evid, Some("o destino é a base do projeto"));
            }
        }
        None => evid.push(ev(
            "perfil sem flow.base (ou sem perfil): só os nomes comuns de tronco e a API",
            "hades project get flow.base",
        )),
    }
    if TRONCOS.contains(&dest.as_str()) {
        return verdict(Barra, evid, Some(&format!("{dest} é nome de tronco")));
    }
    let Some(slug) = gh_repo_slug(ce) else {
        evid.push(ev(
            "remote.origin.url sem owner/repo: não deu para perguntar ao GitHub",
            "git config remote.origin.url",
        ));
        return verdict(Aviso, evid, Some("proteção da branch não conferida"));
    };
    let api = format!("repos/{slug}/branches/{dest}");
    let r = exec(
        &["gh", "api", &api, "--jq", ".protected"],
        ce,
        &ExecOpts::default(),
    );
    if r.code != Some(0) {
        // branch ainda inexistente no remoto responde 404: não é protegida, mas a falha entra na evidência
        evid.push(ev(
            format!(
                "não consegui ler a proteção de {dest}: {}",
                ou(&r.stderr, "resposta vazia")
            ),
            format!("gh api {api}"),
        ));
        return verdict(Aviso, evid, Some("proteção da branch não conferida"));
    }
    if js_trim(&r.stdout) == "true" {
        evid.push(ev(
            format!("{dest} protegida no GitHub"),
            format!("gh api {api}"),
        ));
        return verdict(Barra, evid, Some("branch protegida"));
    }
    evid.push(ev(
        format!("{dest} não está protegida"),
        format!("gh api {api} --jq .protected"),
    ));
    verdict(Passa, evid, None)
}

// ---------------------------------------------------------------- conta-gh

/// `contaGh` (`src/lib/cerberus-porta.ts:56`).
pub fn conta_gh(ce: &CheckEnv) -> CheckVerdict {
    let user = profile_field(ce, "identity.gh.user");
    let token_env = profile_field(ce, "identity.gh.tokenEnv");
    let perfil = ev(
        format!(
            "perfil declara user={} tokenEnv={}",
            user.as_deref().unwrap_or("-"),
            token_env.as_deref().unwrap_or("-")
        ),
        "hades project get identity.gh.*",
    );
    if user.is_none() && token_env.is_none() {
        return verdict(
            Aviso,
            vec![perfil],
            Some("o perfil não declara a conta do gh: nada com o que comparar a conta que empurra"),
        );
    }
    let mut env = Vec::new();
    if let Some(var) = &token_env {
        let token = ce.var(var).unwrap_or("");
        if token.is_empty() {
            return verdict(
                Barra,
                vec![
                    perfil,
                    ev(
                        format!("variável {var} vazia ou ausente no ambiente"),
                        "ambiente da avaliação",
                    ),
                ],
                Some("o token do perfil não está disponível"),
            );
        }
        env.push(("GH_TOKEN".to_owned(), token.to_owned()));
    }
    let r = exec(
        &["gh", "api", "user", "--jq", ".login"],
        ce,
        &ExecOpts {
            env,
            ..ExecOpts::default()
        },
    );
    let login = js_trim(&r.stdout);
    if r.code != Some(0) || login.is_empty() {
        return verdict(
            Aviso,
            vec![
                perfil,
                ev(
                    format!(
                        "não consegui ler a conta do gh: {}",
                        ou(&r.stderr, "resposta vazia")
                    ),
                    "gh api user --jq .login",
                ),
            ],
            Some("conta do gh não conferida"),
        );
    }
    let onde = match &token_env {
        Some(var) => format!("gh api user --jq .login (com o token de {var})"),
        None => "gh api user --jq .login".to_owned(),
    };
    if let Some(user) = &user
        && login.to_lowercase() != user.to_lowercase()
    {
        return verdict(
            Barra,
            vec![
                perfil,
                ev(
                    format!("conta que empurra = {login}, perfil declara {user}"),
                    &onde,
                ),
            ],
            Some("conta do gh diferente da do perfil"),
        );
    }
    verdict(
        Passa,
        vec![perfil, ev(format!("conta que empurra = {login}"), &onde)],
        None,
    )
}

// ---------------------------------------------------------------- nome-dominio

/// `nomeDominio` (`src/lib/cerberus-porta.ts:81`).
pub fn nome_dominio(ce: &CheckEnv) -> CheckVerdict {
    let top = git(ce, &["rev-parse", "--show-toplevel"]);
    let raiz = if top.code == Some(0) {
        PathBuf::from(js_trim(&top.stdout))
    } else {
        ce.dir.clone()
    };
    let script = raiz.join("kit").join("skill-release.sh");
    let (raiz_t, script_t) = (texto_de(&raiz), texto_de(&script));
    if !script.exists() {
        return verdict(
            Passa,
            vec![ev(
                format!(
                    "{raiz_t} não é o núcleo da skill (sem kit/skill-release.sh): a regra de nome e de domínio vale só para ele"
                ),
                &script_t,
            )],
            None,
        );
    }
    let r = exec(
        &["bash", &script_t, "guard", &raiz_t],
        ce,
        &ExecOpts {
            cwd: Some(raiz.clone()),
            timeout_s: Some(GUARD_TIMEOUT_S),
            ..ExecOpts::default()
        },
    );
    let linhas: Vec<String> = split_lines(&format!("{}\n{}", r.stdout, r.stderr))
        .into_iter()
        .filter(|l| !js_trim(l).is_empty())
        .collect();
    let onde = "kit/skill-release.sh guard";
    let evid_of = |max: usize| -> Vec<Evid> {
        linhas
            .iter()
            .take(max)
            .map(|l| ev(js_trim(l), onde))
            .collect()
    };
    let Some(code) = r.code else {
        return verdict(
            Barra,
            vec![ev(format!("a guarda não rodou: {}", r.stderr), onde)],
            Some("nome e domínio não conferidos"),
        );
    };
    if code != 0 {
        let mut evid = evid_of(MAX_EVIDENCE);
        if evid.is_empty() {
            evid.push(ev(format!("saiu {code} sem saída"), onde));
        }
        return verdict(
            Barra,
            evid,
            Some("o núcleo cita nome de projeto ou termo de domínio"),
        );
    }
    if linhas.iter().any(|l| l.contains("nenhum perfil instalado")) {
        return verdict(
            Aviso,
            evid_of(5),
            Some("sem perfil instalado a guarda de nome não tem termos a procurar"),
        );
    }
    let mut evid = evid_of(5);
    if evid.is_empty() {
        evid.push(ev("saiu 0", onde));
    }
    verdict(Passa, evid, None)
}

// ---------------------------------------------------------------- plano-aprovado

/// `isSkillRepo` (`src/lib/cerberus-porta.ts:118`): o diretório avaliado é o git local da skill?
fn is_skill_repo(ce: &CheckEnv) -> bool {
    let args = [
        "git",
        "rev-parse",
        "--path-format=absolute",
        "--git-common-dir",
    ];
    let here = git(ce, &args[1..]);
    let there = exec(
        &args,
        ce,
        &ExecOpts {
            cwd: Some(skill_de(ce)),
            ..ExecOpts::default()
        },
    );
    here.code == Some(0) && there.code == Some(0) && js_trim(&here.stdout) == js_trim(&there.stdout)
}

struct Aprovacao {
    path: String,
    sha: String,
    approved_at: Option<String>,
    by: Option<Value>,
}

/// `readApproval` (`src/lib/cerberus-porta.ts:126`). O texto do erro do parser é o do `serde_json`, não o do Bun.
fn read_approval(file: &Path) -> Result<Aprovacao, String> {
    let bytes = fs::read(file).map_err(|e| e.to_string())?;
    let reg: Value =
        serde_json::from_str(&String::from_utf8_lossy(&bytes)).map_err(|e| e.to_string())?;
    let campo = |key: &str| -> Option<&Value> { reg.as_object().and_then(|o| o.get(key)) };
    match &reg {
        Value::Object(_) | Value::Array(_) => {}
        _ => return Err("registro não é objeto".into()),
    }
    let texto = |key: &str| -> Result<String, String> {
        match campo(key) {
            Some(Value::String(s)) => Ok(s.clone()),
            None => Err(format!("'{key}'")),
            Some(_) => Err(format!("{key} não é texto")),
        }
    };
    let path = texto("path")?;
    let sha = texto("sha")?;
    Ok(Aprovacao {
        path,
        sha,
        approved_at: campo("approvedAt")
            .and_then(Value::as_str)
            .map(str::to_owned),
        by: campo("by").cloned(),
    })
}

/// Valor verdadeiro do JS.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `approvedBy` (`src/lib/cerberus-porta.ts:134`).
fn approved_by(by: Option<&Value>) -> String {
    match by {
        Some(Value::Object(who)) => {
            let name = [who.get("user"), who.get("usuario")]
                .into_iter()
                .find(|v| truthy(*v))
                .flatten()
                .map(|v| js_string(Some(v)))
                .unwrap_or_else(|| "?".into());
            let origem = match who.get("origem") {
                None | Some(Value::Null) => "origem não registrada".to_owned(),
                Some(v) => js_string(Some(v)),
            };
            format!(" por {name} ({origem})")
        }
        Some(Value::Array(_)) => " por ? (origem não registrada)".to_owned(),
        v if truthy(v) => format!(" por {}", js_string(v)),
        _ => String::new(),
    }
}

fn arquivo_regular(p: &Path) -> bool {
    fs::metadata(p).is_ok_and(|m| m.is_file())
}

/// `planoAprovado` (`src/lib/cerberus-porta.ts:143`).
pub fn plano_aprovado(ce: &CheckEnv) -> CheckVerdict {
    let file = match ce.ctx("plano") {
        Some(plano) => PathBuf::from(plano),
        None => {
            let url = git(ce, &["config", "--get", "remote.origin.url"]);
            let m = if url.code == Some(0) {
                owner_repo(js_trim(&url.stdout))
            } else {
                None
            };
            let raiz = work_root(&ce.hades_env());
            // plano da skill: raiz de trabalho própria (_skill), a que o skill-release procura
            let pasta = match m {
                Some((o, r)) if !is_skill_repo(ce) => format!("{o}__{r}"),
                _ => "_skill".to_owned(),
            };
            raiz.join(pasta).join("orchestration").join("plan.json")
        }
    };
    let file_t = texto_de(&file);
    let onde = format!("plan.json em {file_t}");
    let procurado = if ce.ctx("plano").is_some() {
        format!("{file_t} (--ctx plano=)")
    } else {
        file_t.clone()
    };
    if !arquivo_regular(&file) {
        let momento = ou(&ce.momento, "o momento");
        return verdict(
            Barra,
            vec![ev(
                format!("não existe registro de plano aprovado (procurei {procurado})"),
                "hades agent plan-approve <plan.md>",
            )],
            Some(&format!("sem o ok do dono registrado para {momento}")),
        );
    }
    let reg = match read_approval(&file) {
        Ok(reg) => reg,
        Err(msg) => {
            return verdict(
                Barra,
                vec![ev(
                    format!("plan.json ilegível ou sem path/sha: {msg}"),
                    &onde,
                )],
                Some("registro de aprovação inválido"),
            );
        }
    };
    let quando = reg.approved_at.as_deref().unwrap_or("?");
    let plano = Path::new(&reg.path);
    if !arquivo_regular(plano) {
        return verdict(
            Barra,
            vec![ev(
                format!("o plano aprovado {} não existe mais", reg.path),
                &onde,
            )],
            Some("plano aprovado sumiu"),
        );
    }
    let texto = fs::read(plano)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    if plan_sha(&texto) != reg.sha {
        return verdict(
            Barra,
            vec![ev(
                format!("o plano {} mudou depois da aprovação de {quando}", reg.path),
                &onde,
            )],
            Some("aprove de novo com o dono (plan-approve)"),
        );
    }
    let por = approved_by(reg.by.as_ref());
    let detalhe = por
        .is_empty()
        .then_some("o registro não guarda quem aprovou (só o dono roda plan-approve)");
    verdict(
        Passa,
        vec![ev(
            format!(
                "plano {} aprovado em {quando}{por}, íntegro (sha confere)",
                reg.path
            ),
            &onde,
        )],
        detalhe,
    )
}

// ---------------------------------------------------------------- remote-push

/// `remotePush` (`src/lib/cerberus-porta.ts:177`).
pub fn remote_push(ce: &CheckEnv) -> CheckVerdict {
    let bi = branch_info(ce);
    if bi.branch.is_empty() {
        return verdict(
            Barra,
            vec![ev(
                "HEAD destacado: sem remote de push",
                "git symbolic-ref HEAD",
            )],
            None,
        );
    }
    let remote = bi.push_remote.as_str();
    let cfg_where = "git config branch.<b>.pushRemote, remote.pushDefault, branch.<b>.remote";
    if remote.is_empty() {
        return verdict(
            Aviso,
            vec![ev(
                format!(
                    "branch {} sem remote configurado (sem pushRemote, pushDefault nem upstream)",
                    bi.branch
                ),
                cfg_where,
            )],
            Some("o remote só se define no comando do push: confira-o"),
        );
    }
    if remote == NO_PUSH {
        return verdict(
            Barra,
            vec![ev(
                format!("remote de push da branch {} = {NO_PUSH}", bi.branch),
                cfg_where,
            )],
            Some("push desligado nesta branch"),
        );
    }
    let urls = git(ce, &["remote", "get-url", "--push", "--all", remote]);
    let onde = format!("git remote get-url --push {remote}");
    if urls.code != Some(0) {
        return verdict(
            Barra,
            vec![ev(
                format!(
                    "remote {remote} não existe: {}",
                    ou(&urls.stderr, "sem url de push")
                ),
                &onde,
            )],
            None,
        );
    }
    let lista: Vec<&str> = urls
        .stdout
        .split(js_space)
        .filter(|u| !u.is_empty())
        .collect();
    if let Some(bad) = lista.iter().find(|u| u.starts_with(NO_PUSH)) {
        return verdict(
            Barra,
            vec![ev(format!("url de push do remote {remote} = {bad}"), &onde)],
            Some("remote travado para push"),
        );
    }
    verdict(
        Passa,
        vec![ev(
            format!(
                "remote de push = {remote}, url de push {}",
                lista.first().copied().unwrap_or("?")
            ),
            &onde,
        )],
        None,
    )
}

// ---------------------------------------------------------------- segredo-diff

/// `\s` do JS (sem o U+0085 do `\s` Unicode do `regex`, com o BOM).
const JS_S: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// Traduz um padrão do JS: `\s` vira a classe do JS e, com `ci`, letra ASCII fora de classe vira `[aA]` (o `i` do JS
/// sem `u` não dobra `ſ` nem o sinal de Kelvin, que o `(?i)` do `regex` dobraria).
fn js_regex(pat: &str, ci: bool) -> Regex {
    let mut out = String::new();
    let mut em_classe = false;
    let mut chars = pat.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('s') if em_classe => out.push_str(JS_S),
                Some('s') => out.push_str(&format!("[{JS_S}]")),
                Some(e) => {
                    out.push('\\');
                    out.push(e);
                }
                None => out.push('\\'),
            },
            '[' => {
                em_classe = true;
                out.push(c);
            }
            ']' => {
                em_classe = false;
                out.push(c);
            }
            c if ci && !em_classe && c.is_ascii_alphabetic() => {
                out.push_str(&format!(
                    "[{}{}]",
                    c.to_ascii_lowercase(),
                    c.to_ascii_uppercase()
                ));
            }
            c => out.push(c),
        }
    }
    Regex::new(&out).expect("padrão fixo")
}

/// `SECRET_PATTERNS` (`src/lib/cerberus-porta.ts:200`). O marcador da chave privada é montado em partes, como no TS:
/// este arquivo não casa com a própria varredura de segredos do release.
static SECRET_PATTERNS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    let chave = format!("-----BEGIN [A-Z ]*{} KEY-----", "PRIVATE");
    vec![
        (
            "token do GitHub",
            js_regex(
                r"gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}",
                false,
            ),
        ),
        ("chave da AWS", js_regex(r"(AKIA|ASIA)[0-9A-Z]{16}", false)),
        (
            "token do Slack",
            js_regex(r"xox[abposr]-[A-Za-z0-9-]{10,}", false),
        ),
        (
            "chave de API (sk-)",
            js_regex(r"sk-[A-Za-z0-9_-]{32,}", false),
        ),
        ("token do Google", js_regex(r"AIza[0-9A-Za-z_-]{35}", false)),
        ("chave privada", js_regex(&chave, false)),
        (
            "credencial na URL",
            js_regex(r"[a-z][a-z0-9+.-]*://[^/\s:@]+:[^/\s:@]{6,}@[^/\s]+", false),
        ),
        (
            "atribuição de segredo",
            js_regex(
                r#"(secret|token|passw(or)?d|api[_-]?key)["']?\s*[:=]\s*["'][^"'\s]{16,}["']"#,
                true,
            ),
        ),
    ]
});

/// `FALSE_POSITIVE` (`src/lib/cerberus-porta.ts:211`): valor que declara ser exemplo não é segredo.
static FALSE_POSITIVE: LazyLock<Regex> = LazyLock::new(|| {
    js_regex(
        r"exemplo|example|placeholder|changeme|xxxx|<[^>]+>|\$\{|\$\(|your[_-]|\*{4}",
        true,
    )
});

/// `mask` (`src/lib/cerberus-porta.ts:213`).
fn mask(text: &str) -> String {
    let prefixo: String = text.chars().take(MASK_PREFIX).collect();
    format!("{prefixo}…({} caracteres)", text.chars().count())
}

/// `secretIn` (`src/lib/cerberus-porta.ts:217`): o primeiro padrão que casa, se o trecho não for exemplo.
fn secret_in(line: &str) -> Option<(&'static str, String)> {
    SECRET_PATTERNS.iter().find_map(|(name, re)| {
        let m = re.find(line)?;
        (!FALSE_POSITIVE.is_match(m.as_str())).then(|| (*name, m.as_str().to_owned()))
    })
}

/// `pushRange` (`src/lib/cerberus-porta.ts:225`): o intervalo do que o push leva e o porquê.
fn push_range(ce: &CheckEnv) -> Option<(String, String)> {
    if let Some(range) = ce.ctx("range") {
        return Some((
            range.to_owned(),
            "intervalo do contexto (--ctx range=)".to_owned(),
        ));
    }
    let up = git(
        ce,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    if up.code == Some(0) {
        return Some((
            "@{upstream}..HEAD".to_owned(),
            "o que o push leva: @{upstream}..HEAD".to_owned(),
        ));
    }
    let tag = git(ce, &["describe", "--tags", "--abbrev=0"]);
    let tag_t = js_trim(&tag.stdout);
    if tag.code == Some(0) && !tag_t.is_empty() {
        return Some((
            format!("{tag_t}..HEAD"),
            format!("desde a última tag ({tag_t})"),
        ));
    }
    let base = profile_field(ce, "flow.base");
    let mut candidatos = Vec::new();
    if let Some(b) = &base {
        candidatos.push(format!("origin/{b}"));
        candidatos.push(b.clone());
    }
    candidatos.extend(["origin/main", "main", "origin/master", "master"].map(str::to_owned));
    candidatos.into_iter().find_map(|b| {
        let existe = git(ce, &["rev-parse", "--verify", "-q", &b]).code == Some(0);
        (existe && git(ce, &["rev-parse", "HEAD"]).stdout != git(ce, &["rev-parse", &b]).stdout)
            .then(|| (format!("{b}..HEAD"), format!("desde a base {b}")))
    })
}

/// Ordem do `<` e do `sort()` do JS: unidades UTF-16.
fn cmp_js(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `listFiles` (`src/lib/cerberus-porta.ts:242`): arquivos regulares sob a raiz, sem `.git`, em ordem. Pasta que não
/// lê fica de fora (no TS a exceção derruba o check).
fn list_files(root: &Path) -> Vec<String> {
    if arquivo_regular(root) {
        return vec![texto_de(root)];
    }
    fn walk(dir: &Path, found: &mut Vec<String>) {
        let Ok(rd) = fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by(|a, b| {
            cmp_js(
                &a.file_name().to_string_lossy(),
                &b.file_name().to_string_lossy(),
            )
        });
        for entry in entries {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                if entry.file_name() != ".git" {
                    walk(&path, found);
                }
            } else if ft.is_file() {
                found.push(texto_de(&path));
            }
        }
    }
    let mut found = Vec::new();
    walk(root, &mut found);
    found.sort_by(|a, b| cmp_js(a, b));
    found
}

/// O `code` do erro do Node para os casos de leitura comuns.
fn node_code(e: &io::Error) -> &'static str {
    match e.kind() {
        io::ErrorKind::NotFound => "ENOENT",
        io::ErrorKind::PermissionDenied => "EACCES",
        io::ErrorKind::IsADirectory => "EISDIR",
        _ => "erro",
    }
}

/// `scanContent` (`src/lib/cerberus-porta.ts:257`).
fn scan_content(alvo: &str, ctx_key: &str) -> CheckVerdict {
    if !Path::new(alvo).exists() {
        return verdict(
            Barra,
            vec![ev(
                format!("{alvo} não existe: não há como conferir o que sai"),
                format!("--ctx {ctx_key}="),
            )],
            Some("conteúdo a sair não encontrado"),
        );
    }
    let onde = format!("conteúdo de {alvo}");
    let mut files = Vec::new();
    for path in list_files(Path::new(alvo))
        .into_iter()
        .take(MAX_SCANNED_FILES)
    {
        match fs::read(&path) {
            Ok(bytes) => {
                let lines = split_lines(&String::from_utf8_lossy(&bytes));
                files.push((path, lines));
            }
            Err(e) => {
                return verdict(
                    Barra,
                    vec![ev(
                        format!("não consegui ler {path}: {}", node_code(&e)),
                        &path,
                    )],
                    Some("conteúdo a sair ilegível"),
                );
            }
        }
    }
    let mut found = Vec::new();
    for (path, lines) in &files {
        for (i, line) in lines.iter().enumerate() {
            if let Some((name, m)) = secret_in(line) {
                found.push(ev(
                    format!("{name} em {path}:{}: {}", i + 1, mask(&m)),
                    &onde,
                ));
            }
        }
    }
    if !found.is_empty() {
        let n = found.len();
        found.truncate(MAX_EVIDENCE);
        return verdict(
            Barra,
            found,
            Some(&format!(
                "{n} linha(s) com padrão de segredo no que sai; tratar como credencial queimada se já foi publicado"
            )),
        );
    }
    let total: usize = files.iter().map(|(_, l)| l.len()).sum();
    verdict(
        Passa,
        vec![ev(
            format!(
                "{total} linha(s) em {} arquivo(s) sem padrão de token ou chave ({} padrões)",
                files.len(),
                SECRET_PATTERNS.len()
            ),
            &onde,
        )],
        None,
    )
}

/// `/\+(\d+)/` do cabeçalho do hunk: o número depois do primeiro `+` seguido de dígito, menos 1.
fn linha_do_hunk(line: &str) -> i64 {
    let b = line.as_bytes();
    (0..b.len())
        .find(|&i| b[i] == b'+' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
        .map(|i| {
            let digitos: String = line[i + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digitos.parse::<i64>().unwrap_or(i64::MAX) - 1
        })
        .unwrap_or(0)
}

/// `scanDiff` (`src/lib/cerberus-porta.ts:284`).
fn scan_diff(ce: &CheckEnv) -> CheckVerdict {
    let Some((range, why)) = push_range(ce) else {
        return verdict(
            Aviso,
            vec![ev(
                "sem upstream, tag nem base para delimitar o que sai",
                "git rev-parse @{upstream}, git describe --tags, base do perfil",
            )],
            Some("diff que sai não delimitado: passe --ctx range=<a>..<b>"),
        );
    };
    let onde = format!("git diff {range}");
    let d = git(
        ce,
        &["diff", "--no-color", "--unified=0", "--no-ext-diff", &range],
    );
    if d.code != Some(0) {
        return verdict(
            Barra,
            vec![ev(format!("git diff {range} falhou: {}", d.stderr), &onde)],
            Some("não consegui ler o diff que sai"),
        );
    }
    let mut found = Vec::new();
    let mut file = String::new();
    let mut line_no: i64 = 0;
    let lines = split_lines(&d.stdout);
    for line in &lines {
        if let Some(rest) = line.strip_prefix("+++ ") {
            file = rest.strip_prefix("b/").unwrap_or(rest).to_owned();
        } else if line.starts_with("@@") {
            line_no = linha_do_hunk(line);
        } else if let Some(added) = line.strip_prefix('+') {
            line_no = line_no.saturating_add(1);
            if let Some((name, m)) = secret_in(added) {
                found.push(ev(
                    format!("{name} em {file}:{line_no}: {}", mask(&m)),
                    &onde,
                ));
            }
        }
    }
    if !found.is_empty() {
        let n = found.len();
        found.truncate(MAX_EVIDENCE);
        return verdict(
            Barra,
            found,
            Some(&format!(
                "{n} linha(s) com padrão de segredo no que sai ({why}); tratar como credencial queimada se já foi commitado"
            )),
        );
    }
    verdict(
        Passa,
        vec![ev(
            format!(
                "{} linha(s) de diff sem padrão de token ou chave ({} padrões)",
                lines.len(),
                SECRET_PATTERNS.len()
            ),
            format!("{onde} ({why})"),
        )],
        None,
    )
}

/// `segredoDiff` (`src/lib/cerberus-porta.ts:314`): o conteúdo de `texto`/`arquivo` do contexto, ou o diff que sai.
pub fn segredo_diff(ce: &CheckEnv) -> CheckVerdict {
    match (ce.ctx("texto"), ce.ctx("arquivo")) {
        (Some(alvo), _) => scan_content(alvo, "texto"),
        (None, Some(alvo)) => scan_content(alvo, "arquivo"),
        (None, None) => scan_diff(ce),
    }
}

// ---------------------------------------------------------------- tunel-acesso

/// `hostOf` (`src/lib/cerberus-porta.ts:321`): o `hostname` do `URL` do WHATWG, sobre `http://x`; vazio se não lê.
fn host_of(raw: &str) -> String {
    let entrada = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("//{raw}")
    };
    let base = url::Url::parse("http://x").expect("base fixa");
    base.join(&entrada)
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
        .unwrap_or_default()
}

/// `tunelAcesso` (`src/lib/cerberus-porta.ts:329`). A configuração vem de [`config`] com o ambiente do check; ela
/// não falha, então o ramo "não consegui ler a configuração" do TS não existe aqui.
pub fn tunel_acesso(ce: &CheckEnv) -> CheckVerdict {
    let mut op = Opcoes::novo(ce.hades_env(), ce.dir.clone());
    op = op.com_skill(skill_de(ce));
    let cfg = config(&op, true);
    let lista: Vec<String> = cfg
        .get("HADES_CERBERUS_PORTA_TUNEL_HOSTS")
        .map(String::as_str)
        .unwrap_or("")
        .split(',')
        .map(|h| js_trim(h).to_lowercase())
        .filter(|h| !h.is_empty())
        .collect();
    let chave = "Configurações → Cerberus → HADES_CERBERUS_PORTA_TUNEL_HOSTS";
    let host = host_of(js_trim(
        ce.ctx.get("host").map(String::as_str).unwrap_or(""),
    ));
    if host.is_empty() {
        return verdict(
            Barra,
            vec![ev(
                "sem host no contexto",
                "CERBERUS_CTX (--ctx host=<host ou URL do túnel>)",
            )],
            Some("host do túnel não declarado"),
        );
    }
    if lista.is_empty() {
        return verdict(
            Barra,
            vec![ev(
                format!("lista de hosts permitidos vazia; host do túnel = {host}"),
                chave,
            )],
            Some("sem lista permitida o túnel fecha em vez de abrir"),
        );
    }
    if !lista.iter().any(|p| fnmatch_case(&host, p)) {
        return verdict(
            Barra,
            vec![ev(
                format!("host {host} fora da lista ({})", lista.join(", ")),
                chave,
            )],
            Some("host do túnel não permitido"),
        );
    }
    let mut evid = vec![ev(format!("host {host} na lista"), chave)];
    let mut acesso = js_trim(ce.ctx.get("acesso").map(String::as_str).unwrap_or("")).to_lowercase();
    match ce.ctx("tunel") {
        Some(tunel) if acesso.is_empty() => {
            let r = exec(
                &["devtunnel", "access", "list", tunel],
                ce,
                &ExecOpts::default(),
            );
            let onde = format!("devtunnel access list {tunel}");
            if r.code != Some(0) {
                evid.push(ev(
                    format!(
                        "não consegui ler o acesso do túnel {tunel}: {}",
                        ou(&r.stderr, "saída vazia")
                    ),
                    &onde,
                ));
                return verdict(Aviso, evid, Some("autenticação do túnel não conferida"));
            }
            acesso = if r.stdout.to_lowercase().contains("anonymous") {
                "anonimo"
            } else {
                "autenticado"
            }
            .to_owned();
            evid.push(ev(format!("acesso lido do túnel {tunel}: {acesso}"), &onde));
        }
        _ => evid.push(ev(
            format!(
                "acesso declarado pelo chamador: {}",
                ou(&acesso, "não declarado")
            ),
            "CERBERUS_CTX (--ctx acesso=)",
        )),
    }
    if acesso == "autenticado" {
        return verdict(Passa, evid, None);
    }
    if ["anonimo", "anônimo", "anonymous"].contains(&acesso.as_str()) {
        return verdict(Barra, evid, Some("o túnel aceita acesso anônimo"));
    }
    verdict(
        Barra,
        evid,
        Some(
            "sem declarar que o acesso exige autenticação (--ctx acesso=autenticado, ou tunel=<id> para ler)",
        ),
    )
}

// ---------------------------------------------------------------- upstream-push

/// `upstreamPush` (`src/lib/cerberus-porta.ts:363`).
pub fn upstream_push(ce: &CheckEnv) -> CheckVerdict {
    let bi = branch_info(ce);
    if bi.branch.is_empty() {
        return verdict(
            Barra,
            vec![ev(
                "HEAD destacado: sem branch para empurrar",
                "git symbolic-ref HEAD",
            )],
            Some("push de HEAD destacado"),
        );
    }
    let r = head_ref(ce);
    let onde = format!(
        "git config branch.<b>.remote/.merge em {}",
        ou(ce.var("CERBERUS_DIR").unwrap_or(""), ".")
    );
    let rastreio = if bi.merge.is_empty() {
        String::new()
    } else {
        format!("{}/{}", bi.remote, bi.merge)
    };
    let (Some(head), Some(head_ev)) = (r.head, r.head_ev) else {
        let local = if rastreio.is_empty() {
            format!("branch local {} sem upstream", bi.branch)
        } else {
            format!("branch local {}, upstream {rastreio}", bi.branch)
        };
        let mut evid: Vec<Evid> = r.error.into_iter().collect();
        evid.push(ev(local, &onde));
        return verdict(
            Aviso,
            evid,
            Some("sem a PR do contexto não há com o que comparar o upstream"),
        );
    };
    if bi.merge.is_empty() {
        if bi.branch == head {
            return verdict(
                Passa,
                vec![
                    head_ev,
                    ev(
                        format!("branch local {} sem upstream, mesmo nome da PR", bi.branch),
                        &onde,
                    ),
                ],
                Some("sem upstream: o primeiro push cria a branch da PR"),
            );
        }
        return verdict(
            Barra,
            vec![
                head_ev,
                ev(
                    format!(
                        "branch local {} sem upstream, nome diferente de {head}",
                        bi.branch
                    ),
                    &onde,
                ),
            ],
            Some("push -u criaria outra branch que não é a da PR"),
        );
    }
    if bi.merge == head {
        return verdict(
            Passa,
            vec![head_ev, ev(format!("@{{upstream}} = {rastreio}"), &onde)],
            None,
        );
    }
    verdict(
        Barra,
        vec![
            ev(
                format!("@{{upstream}} = {rastreio}, headRefName da PR = {head}"),
                &onde,
            ),
            head_ev,
        ],
        Some("a branch local rastreia outra branch"),
    )
}

/// `PORTA_GUARDS` (`src/lib/cerberus-porta.ts:383`), na ordem do TS.
pub const GUARDAS: [(&str, Guarda); 8] = [
    ("branch-protegida", branch_protegida),
    ("conta-gh", conta_gh),
    ("nome-dominio", nome_dominio),
    ("plano-aprovado", plano_aprovado),
    ("remote-push", remote_push),
    ("segredo-diff", segredo_diff),
    ("tunel-acesso", tunel_acesso),
    ("upstream-push", upstream_push),
];

/// A guarda da Porta pelo id.
pub fn guarda(id: &str) -> Option<Guarda> {
    GUARDAS.iter().find(|(n, _)| *n == id).map(|(_, g)| *g)
}

#[cfg(test)]
mod tests;
