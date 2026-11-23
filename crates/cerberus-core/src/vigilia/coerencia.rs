//! As 2 guardas de coerência da Vigília (`src/lib/cerberus-vigilia.ts:807-1083`): o registro do orquestrador confere
//! com o git da worktree, as sessões do herdr e as PRs do GitHub (`registro`), e o "Estado atual" do plano aprovado
//! confere com o registro (`plano`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::LazyLock;

use fancy_regex::Regex as Fancy;
use hades_core::config::split_lines;
use regex::Regex;
use serde_json::{Map, Value};

use super::{
    Achados, Agente, Indisponivel, LEITURA_MAX, Vigilia, W, alvos_ou_falha, curto, is_dir, is_file,
    js_basename, js_join, jsons_in, ler, tentar, texto,
};
use crate::modelo::Resultado::{Aviso, Barra, Ok as Passa};
use crate::modelo::{js_norm, js_string, js_trim};

type Dict = Map<String, Value>;

const SHA: &str = "[0-9a-f]{7,40}";
/// `SEM_PR` (`src/lib/cerberus-vigilia.ts:809`): modos sem PR a conferir.
const SEM_PR: [&str; 2] = ["skill", "review"];

static SHA_FIXO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("^{SHA}$")).expect("regex"));

fn fancy(source: &str) -> Fancy {
    Fancy::new(source).expect("regex da Vigília")
}

/// Valor "verdadeiro" do JS.
fn truthy(v: Option<&Value>) -> bool {
    !texto(v).is_empty() || matches!(v, Some(Value::Array(_) | Value::Object(_)))
}

/// `===` do JS entre um valor do JSON e um texto.
fn igual(v: Option<&Value>, s: &str) -> bool {
    matches!(v, Some(Value::String(x)) if x == s)
}

// ---------------------------------------------------------------- registro × git, herdr e GitHub

/// `conferirBase` (`src/lib/cerberus-vigilia.ts:811`): sha fixo tem de ser ancestral de HEAD; ref móvel, ter ancestral
/// comum com HEAD.
fn conferir_base(
    v: &Vigilia,
    a: &Agente,
    wt: &str,
    base: &str,
    achados: &mut Achados,
) -> Result<(), Indisponivel> {
    let fixa = SHA_FIXO.is_match(base);
    let alvo = format!("{base}^{{commit}}");
    if v.git(wt, &["rev-parse", "--verify", "-q", &alvo])?.0 != 0 {
        achados.add(
            if fixa { Barra } else { Aviso },
            format!(
                "{}: base {base} do registro não existe no repositório da worktree",
                a.nome
            ),
            format!("git -C {wt} rev-parse --verify {base}"),
        );
        return Ok(());
    }
    if fixa {
        let onde = format!("git -C {wt} merge-base --is-ancestor {base} HEAD");
        if v.git(wt, &["merge-base", "--is-ancestor", base, "HEAD"])?.0 == 0 {
            achados.add(
                Passa,
                format!("{}: base {base} é ancestral de HEAD", a.nome),
                onde,
            );
        } else {
            achados.add(
                Barra,
                format!(
                    "{}: base {base} do registro não é ancestral de HEAD (rebase ou base trocada)",
                    a.nome
                ),
                onde,
            );
        }
        return Ok(());
    }
    let (rc, mb) = v.git(wt, &["merge-base", base, "HEAD"])?;
    let onde = format!("git -C {wt} merge-base {base} HEAD");
    if rc == 0 {
        let curto_mb: String = js_trim(&mb).chars().take(12).collect();
        achados.add(
            Passa,
            format!(
                "{}: base {base} (ref móvel) tem ancestral comum {curto_mb} com HEAD",
                a.nome
            ),
            onde,
        );
    } else {
        achados.add(
            Barra,
            format!(
                "{}: a branch não tem ancestral comum com a base {base}",
                a.nome
            ),
            onde,
        );
    }
    Ok(())
}

/// `conferirGit` (`src/lib/cerberus-vigilia.ts:829`): a worktree existe, está na branch do registro e a base confere.
fn conferir_git(v: &Vigilia, a: &Agente, achados: &mut Achados) {
    let wt = a.worktree();
    if wt.is_empty() || !is_dir(&wt) {
        achados.add(
            Barra,
            format!("{}: worktree do registro não existe: {wt}", a.nome),
            a.arquivo.clone(),
        );
        return;
    }
    let r = (|| {
        let (rc, br_raw) = v.git(&wt, &["branch", "--show-current"])?;
        let br = js_trim(&br_raw);
        let modo = a.modo();
        if modo != "review" {
            let want = a.campo("branch");
            if rc != 0 || br != want {
                let want = if want.is_empty() { "(vazia)" } else { &want };
                let br = if br.is_empty() { "HEAD destacado" } else { br };
                achados.add(
                    Barra,
                    format!(
                        "{}: registro diz branch {want}, a worktree está em {br}",
                        a.nome
                    ),
                    format!("git -C {wt} branch --show-current · {}", a.arquivo),
                );
            } else {
                achados.add(
                    Passa,
                    format!("{}: branch {br} confere com o registro", a.nome),
                    format!("git -C {wt} branch --show-current"),
                );
            }
        }
        let base = a.campo("base");
        if !base.is_empty() && modo != "review" {
            conferir_base(v, a, &wt, &base, achados)?;
        }
        Ok(())
    })();
    tentar(achados, r);
}

/// `sessoesHerdr` (`src/lib/cerberus-vigilia.ts:851`): os objetos de `result.agents` do `herdr agent list`.
pub fn sessoes_herdr(v: &Vigilia) -> Result<Vec<Dict>, Indisponivel> {
    let (rc, out) = v.rodar(&["herdr", "agent", "list"], None)?;
    if rc != 0 {
        return Err(Indisponivel::novo(
            format!("herdr agent list saiu {rc}"),
            "herdr agent list",
        ));
    }
    let ilegivel = || {
        Indisponivel::novo(
            "herdr agent list sem result.agents legível",
            "herdr agent list",
        )
    };
    let doc: Value = serde_json::from_str(&out).map_err(|_| ilegivel())?;
    let Some(Value::Array(agents)) = doc
        .as_object()
        .and_then(|d| d.get("result"))
        .and_then(Value::as_object)
        .and_then(|r| r.get("agents"))
    else {
        return Err(ilegivel());
    };
    Ok(agents
        .iter()
        .filter_map(|x| x.as_object().cloned())
        .collect())
}

/// `conferirHerdr` (`src/lib/cerberus-vigilia.ts:864`): agente ativo tem sessão viva pelo nome; sessão no painel ou
/// na worktree sem o nome é órfã.
fn conferir_herdr(a: &Agente, sessoes: &[Dict], achados: &mut Achados) {
    if !a.ativo() {
        achados.add(
            Passa,
            format!(
                "{}: registro em {}, sessão no herdr não exigida",
                a.nome,
                a.campo("state")
            ),
            a.arquivo.clone(),
        );
        return;
    }
    let pane = a.campo("pane");
    let wt = a.worktree();
    if let Some(s) = sessoes.iter().find(|s| igual(s.get("name"), &a.nome)) {
        let pid = s.get("pane_id");
        if !pane.is_empty() && truthy(pid) && !igual(pid, &pane) {
            achados.add(
                Aviso,
                format!(
                    "{}: no herdr no painel {}, o registro diz {pane}",
                    a.nome,
                    texto(pid)
                ),
                "herdr agent list",
            );
        } else {
            let ou = |t: String| if t.is_empty() { "?".to_owned() } else { t };
            achados.add(
                Passa,
                format!(
                    "{}: sessão viva no herdr ({}, {})",
                    a.nome,
                    ou(texto(s.get("agent_status"))),
                    ou(texto(pid))
                ),
                "herdr agent list",
            );
        }
        return;
    }
    let orfa = sessoes.iter().find(|s| {
        (!pane.is_empty() && igual(s.get("pane_id"), &pane))
            || igual(s.get("cwd"), &wt)
            || igual(s.get("foreground_cwd"), &wt)
    });
    let estado = match a.campo("state") {
        e if e.is_empty() => "ativo".to_owned(),
        e => e,
    };
    match orfa {
        Some(s) => {
            let pid = match texto(s.get("pane_id")) {
                p if p.is_empty() => "?".to_owned(),
                p => p,
            };
            achados.add(
                Barra,
                format!(
                    "{}: registrado como {estado}, mas a sessão do painel {pid} está no herdr sem o nome (nome perdido; herdr agent rename + adopt)",
                    a.nome
                ),
                "herdr agent list",
            );
        }
        None => {
            let p = if pane.is_empty() { "—" } else { &pane };
            achados.add(
                Aviso,
                format!(
                    "{}: registrado como {estado} sem sessão no herdr (nem nome, nem painel {p}, nem cwd na worktree)",
                    a.nome
                ),
                "herdr agent list",
            );
        }
    }
}

/// `conferirPr` (`src/lib/cerberus-vigilia.ts:889`): a PR da branch existe, é a do registro e não foi fechada.
fn conferir_pr(v: &Vigilia, a: &Agente, achados: &mut Achados) -> Result<(), Indisponivel> {
    let modo = a.modo();
    if SEM_PR.contains(&modo.as_str()) {
        achados.add(
            Passa,
            format!("{}: modo {modo}, sem PR a conferir", a.nome),
            a.arquivo.clone(),
        );
        return Ok(());
    }
    let br = a.campo("branch");
    if br.is_empty() {
        achados.add(
            Barra,
            format!("{}: registro sem branch", a.nome),
            a.arquivo.clone(),
        );
        return Ok(());
    }
    let onde = format!("gh pr list --head {br} --state all");
    let wt = a.worktree();
    let argv = [
        "gh",
        "pr",
        "list",
        "--head",
        &br,
        "--state",
        "all",
        "--json",
        "number,state,headRefName",
        "--limit",
        "10",
    ];
    let (rc, out) = v.rodar(&argv, is_dir(&wt).then_some(wt.as_str()))?;
    if rc != 0 {
        return Err(Indisponivel::novo(format!("gh pr list saiu {rc}"), onde));
    }
    let ilegivel = || Indisponivel::novo("gh pr list sem JSON legível", onde.clone());
    let doc = serde_json::from_str::<Value>(&out).map_err(|_| ilegivel())?;
    let Value::Array(itens) = js_norm(doc) else {
        return Err(ilegivel());
    };
    // `null.headRefName` lança no JS; os demais valores só não casam
    if itens.iter().any(Value::is_null) {
        return Err(ilegivel());
    }
    let prs: Vec<&Value> = itens
        .iter()
        .filter(|p| igual(p.get("headRefName"), &br))
        .collect();
    let reg_pr = a.campo("pr");
    if prs.is_empty() {
        let diz = if reg_pr.is_empty() {
            String::new()
        } else {
            format!(" (o registro diz #{reg_pr})")
        };
        achados.add(
            if reg_pr.is_empty() { Aviso } else { Barra },
            format!("{}: nenhuma PR para a branch {br}{diz}", a.nome),
            onde,
        );
        return Ok(());
    }
    let numero = |p: &Value| js_string(p.get("number"));
    let pr = if reg_pr.is_empty() {
        Some(prs[0])
    } else {
        prs.iter().copied().find(|p| numero(p) == reg_pr)
    };
    let Some(pr) = pr else {
        let lista: Vec<String> = prs.iter().map(|p| format!("#{}", numero(p))).collect();
        achados.add(
            Barra,
            format!(
                "{}: registro aponta a PR #{reg_pr}, a branch {br} tem {}",
                a.nome,
                lista.join(", ")
            ),
            onde,
        );
        return Ok(());
    };
    let n = numero(pr);
    let st = pr.get("state");
    if igual(st, "CLOSED") {
        achados.add(
            Barra,
            format!("{}: PR #{n} fechada sem merge", a.nome),
            onde,
        );
    } else if igual(st, "MERGED") && a.ativo() {
        let estado = match a.campo("state") {
            e if e.is_empty() => "ativo".to_owned(),
            e => e,
        };
        achados.add(
            Aviso,
            format!(
                "{}: PR #{n} já mergeada e o registro segue em {estado}",
                a.nome
            ),
            onde,
        );
    } else {
        achados.add(
            Passa,
            format!("{}: PR #{n} {} na branch {br}", a.nome, js_string(st)),
            onde,
        );
    }
    Ok(())
}

/// `gRegistro` (`src/lib/cerberus-vigilia.ts:925`).
pub fn registro(v: &Vigilia, achados: &mut Achados) {
    let lista = alvos_ou_falha(v, achados);
    let sessoes = if lista.is_empty() {
        Some(Vec::new())
    } else {
        match sessoes_herdr(v) {
            Ok(s) => Some(s),
            Err(e) => {
                achados.falta(e);
                None
            }
        }
    };
    for a in &lista {
        conferir_git(v, a, achados);
        if let Some(s) = &sessoes {
            conferir_herdr(a, s, achados);
        }
        let r = conferir_pr(v, a, achados);
        tentar(achados, r);
    }
}

// ---------------------------------------------------------------- plano × registro

/// `PAINEL` (`src/lib/cerberus-vigilia.ts:943`).
static PAINEL: LazyLock<Fancy> =
    LazyLock::new(|| fancy(&format!(r"(?<![{W}])(w[0-9A-Za-z]+:p[0-9]+)(?![{W}])")));
/// `PORTA` (`src/lib/cerberus-vigilia.ts:944`).
static PORTA: LazyLock<Fancy> =
    LazyLock::new(|| fancy(&format!(r"(?i)(?<![{W}])porta\s+([0-9]{{2,5}})(?![{W}])")));
/// `PR_NUM` (`src/lib/cerberus-vigilia.ts:945`).
static PR_NUM: LazyLock<Fancy> =
    LazyLock::new(|| fancy(&format!(r"(?<![{W}&])#([0-9]{{1,7}})(?![{W}])")));
/// `SHA_PLANO` (`src/lib/cerberus-vigilia.ts:946`).
static SHA_PLANO: LazyLock<Fancy> =
    LazyLock::new(|| fancy(&format!(r"(?<![{W}/#-])({SHA})(?![{W}-])")));
static COMENTARIO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").expect("regex"));
static ESTADO_ATUAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^## Estado atual\s*$").expect("regex"));
static ITEM_LISTA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[-*]\s").expect("regex"));

/// `estadoAtual` (`src/lib/cerberus-vigilia.ts:949`): linhas da seção "## Estado atual", sem os comentários HTML do
/// modelo; `None` sem a seção (ou com ela vazia).
pub fn estado_atual(texto: &str) -> Option<Vec<String>> {
    let mut sec: Vec<String> = Vec::new();
    let mut dentro = false;
    for ln in split_lines(texto) {
        if ln.starts_with("## ") {
            dentro = ESTADO_ATUAL.is_match(&ln);
            continue;
        }
        if dentro {
            sec.push(ln);
        }
    }
    if sec.is_empty() {
        return None;
    }
    let limpo = COMENTARIO.replace_all(&sec.join("\n"), "").into_owned();
    Some(split_lines(&limpo))
}

/// `planoAtivo` (`src/lib/cerberus-vigilia.ts:962`): o `path` de `<orch>/plan.json`, se houver. Divergência: o texto
/// do erro de leitura e do parser é o do Rust (`serde_json`), não o do Bun.
fn plano_ativo(orch: &str) -> Result<(Option<String>, String), Indisponivel> {
    let pj = js_join(&[orch, "plan.json"]);
    if !is_file(&pj) {
        return Ok((None, pj));
    }
    let lido = fs::read_to_string(&pj)
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
        .and_then(|doc| match doc.get("path") {
            Some(Value::String(p)) => Ok(p.clone()),
            _ => Err("'path'".to_owned()),
        });
    match lido {
        Ok(p) => Ok((Some(p), pj)),
        Err(m) => Err(Indisponivel::novo(format!("plan.json ilegível ({m})"), pj)),
    }
}

/// `linhasDoAgente` (`src/lib/cerberus-vigilia.ts:977`): itens do Estado atual que citam o agente, cada um com as
/// linhas de continuação juntadas por espaço.
pub fn linhas_do_agente(sec: &[String], nome: &str) -> Vec<String> {
    let pat = fancy(&format!(
        r"(?<![{W}-]){}(?![{W}-])",
        fancy_regex::escape(nome)
    ));
    let mut itens = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for ln in sec {
        if ITEM_LISTA.is_match(ln) || js_trim(ln).is_empty() {
            if !cur.is_empty() {
                itens.push(cur.join(" "));
            }
            cur = if js_trim(ln).is_empty() {
                Vec::new()
            } else {
                vec![ln.as_str()]
            };
        } else {
            cur.push(ln);
        }
    }
    if !cur.is_empty() {
        itens.push(cur.join(" "));
    }
    itens
        .into_iter()
        .filter(|i| pat.is_match(i).unwrap_or(false))
        .collect()
}

/// `indexOf` do JS em unidades UTF-16.
fn index_of(hay: &[u16], agulha: &[u16], de: usize) -> Option<usize> {
    if agulha.is_empty() {
        return Some(de.min(hay.len()));
    }
    (de..hay.len().saturating_sub(agulha.len() - 1)).find(|&i| hay[i..].starts_with(agulha))
}

/// `recorte` (`src/lib/cerberus-vigilia.ts:994`): do nome até o próximo agente citado, no máximo `raio` unidades
/// UTF-16 (um item pode listar várias unidades).
pub fn recorte(item: &str, nome: &str, outros: &[String], raio: usize) -> String {
    let h: Vec<u16> = item.encode_utf16().collect();
    let n: Vec<u16> = nome.encode_utf16().collect();
    let Some(i) = index_of(&h, &n, 0) else {
        return item.to_owned();
    };
    let fim = outros
        .iter()
        .filter_map(|o| {
            let o: Vec<u16> = o.encode_utf16().collect();
            index_of(&h, &o, i + n.len())
        })
        .filter(|&j| j > i)
        .fold(i + raio, usize::min)
        .min(h.len());
    String::from_utf16_lossy(&h[i..fim])
}

/// `unicos` (`src/lib/cerberus-vigilia.ts:1001`): os grupos 1 distintos, na ordem em que aparecem.
fn unicos(re: &Fancy, s: &str) -> Vec<String> {
    let mut vistos = BTreeSet::new();
    re.captures_iter(s)
        .filter_map(Result::ok)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_owned()))
        .filter(|x| vistos.insert(x.clone()))
        .collect()
}

/// `pareceSha` (`src/lib/cerberus-vigilia.ts:589`).
fn parece_sha(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_digit()) && s.bytes().any(|b| (b'a'..=b'f').contains(&b))
}

fn ordenados(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// `conferirPlano` (`src/lib/cerberus-vigilia.ts:1003`): o trecho do Estado atual que cita o agente confere painel,
/// porta, PR e commits com o registro.
fn conferir_plano(
    v: &Vigilia,
    a: &Agente,
    sec: &[String],
    plano: &str,
    outros: &[String],
    achados: &mut Achados,
) {
    let itens = linhas_do_agente(sec, &a.nome);
    let onde = format!("{plano} §Estado atual");
    if itens.is_empty() {
        let estado = match a.campo("state") {
            e if e.is_empty() => "ativo".to_owned(),
            e => e,
        };
        achados.add(
            if a.ativo() { Aviso } else { Passa },
            format!(
                "{}: registro em {estado} e o agente não aparece no Estado atual",
                a.nome
            ),
            onde,
        );
        return;
    }
    let trecho = itens
        .iter()
        .map(|i| recorte(i, &a.nome, outros, 200))
        .collect::<Vec<_>>()
        .join(" ");
    let mut problemas = 0;
    let pane = a.campo("pane");
    let paineis = unicos(&PAINEL, &trecho);
    if !pane.is_empty() && !paineis.is_empty() && !paineis.contains(&pane) {
        problemas += 1;
        achados.add(
            Aviso,
            format!(
                "{}: o plano cita o painel {}, o registro diz {pane}",
                a.nome,
                ordenados(paineis).join(", ")
            ),
            onde.clone(),
        );
    }
    let porta = a.campo("port");
    let portas = unicos(&PORTA, &trecho);
    if !porta.is_empty() && !portas.is_empty() && !portas.contains(&porta) {
        problemas += 1;
        achados.add(
            Aviso,
            format!(
                "{}: o plano cita a porta {}, o registro diz {porta}",
                a.nome,
                ordenados(portas).join(", ")
            ),
            onde.clone(),
        );
    }
    let reg_pr = a.campo("pr");
    let prs = unicos(&PR_NUM, &trecho);
    if !reg_pr.is_empty() && !prs.is_empty() && !prs.contains(&reg_pr) {
        problemas += 1;
        achados.add(
            Aviso,
            format!(
                "{}: o plano cita a PR #{}, o registro diz #{reg_pr}",
                a.nome,
                ordenados(prs).join(", #")
            ),
            onde.clone(),
        );
    }
    let wt = a.worktree();
    if is_dir(&wt) {
        for sha in unicos(&SHA_PLANO, &trecho)
            .into_iter()
            .filter(|s| parece_sha(s))
        {
            match v.git(&wt, &["cat-file", "-e", &format!("{sha}^{{commit}}")]) {
                Ok((0, _)) => {}
                Ok(_) => {
                    problemas += 1;
                    achados.add(
                        Aviso,
                        format!(
                            "{}: o plano cita o commit {sha}, que não existe no repositório da worktree",
                            a.nome
                        ),
                        onde.clone(),
                    );
                }
                Err(e) => {
                    achados.falta(e);
                    break;
                }
            }
        }
    }
    if problemas == 0 {
        achados.add(
            Passa,
            format!(
                "{}: citado no Estado atual; painel, porta, PR e commits citados conferem: \"{}\"",
                a.nome,
                curto(&trecho, 140)
            ),
            onde,
        );
    }
}

/// `gPlano` (`src/lib/cerberus-vigilia.ts:1047`): por orquestrador, o plano de `plan.json` contra os agentes alvo.
pub fn plano(v: &Vigilia, achados: &mut Achados) {
    let mut ordem: Vec<String> = Vec::new();
    let mut por_orch: BTreeMap<String, Vec<Agente>> = BTreeMap::new();
    for a in alvos_ou_falha(v, achados) {
        let o = a.orch();
        if !por_orch.contains_key(&o) {
            ordem.push(o.clone());
        }
        por_orch.entry(o).or_default().push(a);
    }
    for orch in ordem {
        let ags = &por_orch[&orch];
        let lido = plano_ativo(&orch).and_then(|(plano, pj)| {
            let texto = match &plano {
                Some(p) => ler(p, LEITURA_MAX)?,
                None => None,
            };
            Ok((plano, pj, texto))
        });
        let Some((plano, pj, texto)) = (match lido {
            Ok(x) => Some(x),
            Err(e) => {
                achados.falta(e);
                None
            }
        }) else {
            continue;
        };
        let Some(plano) = plano else {
            achados.add(
                Passa,
                "sem plano aprovado registrado: nada a confrontar",
                pj,
            );
            continue;
        };
        let Some(texto) = texto else {
            achados.add(
                Aviso,
                format!("plan.json aponta {plano}, que não existe"),
                pj,
            );
            continue;
        };
        let Some(sec) = estado_atual(&texto) else {
            achados.add(Aviso, "plano sem a seção \"## Estado atual\"", plano);
            continue;
        };
        let nomes: Vec<String> = jsons_in(&js_join(&[&orch, "agents"]))
            .iter()
            .map(|f| js_basename(f, ".json"))
            .collect();
        for a in ags {
            if a.modo() == "skill" {
                achados.add(
                    Passa,
                    format!(
                        "{}: modo skill, fora do Estado atual do plano do projeto",
                        a.nome
                    ),
                    plano.clone(),
                );
                continue;
            }
            let outros: Vec<String> = nomes.iter().filter(|n| **n != a.nome).cloned().collect();
            conferir_plano(v, a, &sec, &plano, &outros, achados);
        }
    }
}
