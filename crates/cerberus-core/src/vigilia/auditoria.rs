//! As 6 guardas de auditoria da Vigília (`src/lib/cerberus-vigilia.ts:390-807`): o que o report afirma confere com o
//! disco, o git e o gate-journal.

use std::collections::HashMap;
use std::fs;
use std::sync::LazyLock;

use fancy_regex::Regex as Fancy;
use hades_core::config::split_lines;
use regex::Regex;

use super::{
    Achados, Agente, Indisponivel, LEITURA_MAX, Vigilia, W, alvos_ou_falha, blocos, curto,
    em_cada_repo, errno_texto, is_dir, is_file, js_basename, js_dirname, js_join, ler, ler_cauda,
    real, report_de, report_done, report_path, tentar,
};
use crate::check::fnmatch_case;
use crate::modelo::Resultado::{self, Aviso, Barra, Ok as Passa};
use crate::modelo::js_trim;

/// Bytes do fim de cada `gate-journal.tsv` (`JOURNAL_CAUDA`, `src/lib/cerberus-vigilia.ts:17`).
const JOURNAL_CAUDA: u64 = 4 << 20;
/// Arquivos visitados na busca de captura pelo nome (`BUSCA_MAX`, `src/lib/cerberus-vigilia.ts:490`).
const BUSCA_MAX: usize = 20000;

fn before() -> String {
    format!("(?<![{W}])")
}

fn after() -> String {
    format!("(?![{W}])")
}

fn fancy(source: &str) -> Fancy {
    Fancy::new(source).expect("regex da Vigília")
}

/// `.test` de uma regex com lookaround; erro de execução (limite de backtracking) conta como não casar.
fn testa(re: &Fancy, texto: &str) -> bool {
    re.is_match(texto).unwrap_or(false)
}

/// O grupo 1 (ou 2) de cada casamento, com a posição em bytes do grupo.
fn grupos(re: &Fancy, texto: &str) -> Vec<(usize, String)> {
    re.captures_iter(texto)
        .filter_map(Result::ok)
        .filter_map(|c| {
            c.get(1)
                .or_else(|| c.get(2))
                .map(|m| (m.start(), m.as_str().to_owned()))
        })
        .collect()
}

// ---------------------------------------------------------------- status

/// `gStatus` (`src/lib/cerberus-vigilia.ts:392`).
pub fn status(v: &Vigilia, achados: &mut Achados) {
    for a in alvos_ou_falha(v, achados) {
        let Some(t) = report_de(v, &a, achados) else {
            continue;
        };
        let primeira = if js_trim(&t).is_empty() {
            "(vazio)".to_owned()
        } else {
            split_lines(&t).into_iter().next().unwrap_or_default()
        };
        let p = report_path(v, &a);
        if report_done(Some(&t)) {
            achados.add(
                Passa,
                format!("{}: primeira linha \"{}\"", a.nome, curto(&primeira, 80)),
                p,
            );
        } else {
            achados.add(
                Barra,
                format!(
                    "{}: primeira linha \"{}\" não é STATUS: DONE",
                    a.nome,
                    curto(&primeira, 80)
                ),
                p,
            );
        }
    }
}

// ---------------------------------------------------------------- gates

/// `FILA` (`src/lib/cerberus-vigilia.ts:404`).
static FILA: LazyLock<Fancy> = LazyLock::new(|| {
    let (b, a) = (before(), after());
    fancy(&format!(
        r"(?i)gate-lock|pela fila|\(fila{a}|{b}queue{a}|{b}kind\s*`?(?:lint|build|test|gate)"
    ))
});
/// `POSITIVO` (`src/lib/cerberus-vigilia.ts:405`).
static POSITIVO: LazyLock<Fancy> = LazyLock::new(|| {
    let (b, a) = (before(), after());
    fancy(&format!(
        r"(?i){b}(?:ok|pass(?:a|ou|ed|es)?|verde|green|rc=0|exit 0|sa[ií]da 0){a}|✓|✅"
    ))
});
/// `NEGATIVO` (`src/lib/cerberus-vigilia.ts:406`).
static NEGATIVO: LazyLock<Fancy> = LazyLock::new(|| {
    let (b, a) = (before(), after());
    fancy(&format!(
        r"(?i){b}(?:falhou|failed|fail(?:s|ed)?|vermelho|red|rc=[1-9][0-9]*){a}|(?<!0 )✗|❌"
    ))
});
/// `KIND` (`src/lib/cerberus-vigilia.ts:407`).
static KIND: LazyLock<Fancy> = LazyLock::new(|| {
    let (b, a) = (before(), after());
    fancy(&format!(
        r"(?i){b}run\s+(test:integration|lint|build|test|gate){a}|{b}kind\s*`?(test:integration|lint|build|test|gate){a}"
    ))
});
/// `LABEL` (`src/lib/cerberus-vigilia.ts:408`).
static LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"--label[\s=]+`?([A-Za-z0-9._:/-]+)").expect("regex"));

/// `Alegacao` (`src/lib/cerberus-vigilia.ts:410`): um gate que o report diz ter passado pela fila.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alegacao {
    pub linha: usize,
    pub kind: Option<String>,
    pub label: Option<String>,
    pub trecho: String,
}

/// `alegacoesDeGate` (`src/lib/cerberus-vigilia.ts:412`).
pub fn alegacoes_de_gate(texto: &str) -> Vec<Alegacao> {
    let bs = blocos(texto);
    let mut out = Vec::new();
    for (i, (ini, linhas)) in bs.iter().enumerate() {
        let mut txt = linhas.join(" ");
        if !testa(&FILA, &txt) {
            continue;
        }
        if let Some((_, proximo)) = bs.get(i + 1).filter(|_| linhas[0].starts_with('#')) {
            txt.push(' ');
            txt.push_str(&proximo.join(" "));
        }
        if !testa(&POSITIVO, &txt) || testa(&NEGATIVO, &txt) {
            continue;
        }
        let kind = grupos(&KIND, &txt)
            .into_iter()
            .next()
            .map(|(_, k)| k.to_lowercase());
        let label = LABEL.captures(&txt).map(|c| c[1].to_owned());
        out.push(Alegacao {
            linha: *ini,
            kind,
            label,
            trecho: txt,
        });
    }
    out
}

/// `RegistroGate` (`src/lib/cerberus-vigilia.ts:428`).
#[derive(Clone, Debug)]
struct RegistroGate {
    ts: String,
    kind: String,
    label: String,
    rc: i64,
    arquivo: String,
}

static RC_INTEIRO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*[-+]?[0-9]+\s*$").expect("regex"));

/// `journalDaWorktree` (`src/lib/cerberus-vigilia.ts:431`): linhas `done` do gate-journal de todo repositório cujo
/// clone é a worktree.
fn journal_da_worktree(v: &Vigilia, wt: &str) -> Result<Vec<RegistroGate>, Indisponivel> {
    let alvo = real(wt);
    let mut reais: HashMap<String, String> = HashMap::new();
    let mut out = Vec::new();
    for j in em_cada_repo(&v.work(), "locks/gate-journal.tsv") {
        let t = ler_cauda(&j, JOURNAL_CAUDA).map_err(|e| {
            Indisponivel::novo(format!("gate-journal ilegível ({})", errno_texto(&e)), &j)
        })?;
        for ln in split_lines(&t) {
            let c: Vec<&str> = ln.split('\t').collect();
            if c.len() < 7 || c[1] != "done" || !c[6].starts_with("rc=") {
                continue;
            }
            let r = reais.entry(c[5].to_owned()).or_insert_with(|| real(c[5]));
            if *r != alvo || !RC_INTEIRO.is_match(&c[6][3..]) {
                continue;
            }
            let rc = js_trim(&c[6][3..])
                .trim_start_matches('+')
                .parse::<i64>()
                .unwrap_or(i64::MAX);
            out.push(RegistroGate {
                ts: c[0].to_owned(),
                kind: c[2].to_owned(),
                label: c[3].to_owned(),
                rc,
                arquivo: j.clone(),
            });
        }
    }
    Ok(out)
}

/// `casaKind` (`src/lib/cerberus-vigilia.ts:455`).
fn casa_kind(pedido: Option<&str>, kind: &str) -> bool {
    match pedido {
        None => true,
        Some(p) => kind == p || (p == "test" && kind.starts_with("test")),
    }
}

/// `gGates` (`src/lib/cerberus-vigilia.ts:457`).
pub fn gates(v: &Vigilia, achados: &mut Achados) {
    for a in alvos_ou_falha(v, achados) {
        let Some(t) = report_de(v, &a, achados) else {
            continue;
        };
        let rp = report_path(v, &a);
        let alegs = alegacoes_de_gate(&t);
        if alegs.is_empty() {
            achados.add(
                Passa,
                format!("{}: report não cita gate verde pela fila", a.nome),
                rp,
            );
            continue;
        }
        let wt = a.worktree();
        let regs = match journal_da_worktree(v, &wt) {
            Ok(r) => r,
            Err(e) => {
                achados.falta(e);
                continue;
            }
        };
        let desde = a.campo("createdAt");
        let regs: Vec<RegistroGate> = regs.into_iter().filter(|r| r.ts >= desde).collect();
        for al in alegs {
            let onde = format!("{rp}:{}", al.linha);
            let ult = regs.iter().rfind(|r| {
                casa_kind(al.kind.as_deref(), &r.kind)
                    && al.label.as_ref().is_none_or(|l| r.label == *l)
            });
            let partes: Vec<String> = [
                al.kind.as_ref().map(|k| format!("kind {k}")),
                al.label.as_ref().map(|l| format!("label {l}")),
            ]
            .into_iter()
            .flatten()
            .collect();
            let filtro = if partes.is_empty() {
                "qualquer kind".to_owned()
            } else {
                partes.join(", ")
            };
            match ult {
                None => achados.add(
                    Barra,
                    format!(
                        "{}: \"{}\" sem registro done ({filtro}) da worktree {wt} desde {} no gate-journal",
                        a.nome,
                        curto(&al.trecho, 120),
                        if desde.is_empty() { "o início" } else { &desde }
                    ),
                    onde,
                ),
                Some(u) if u.rc == 0 => achados.add(
                    Passa,
                    format!("{}: gate ({filtro}) com rc=0 em {} (label {})", a.nome, u.ts, u.label),
                    format!("{} · {onde}", u.arquivo),
                ),
                Some(u) => achados.add(
                    Barra,
                    format!(
                        "{}: citado verde, mas o último registro ({filtro}) saiu rc={} em {} (label {})",
                        a.nome, u.rc, u.ts, u.label
                    ),
                    format!("{} · {onde}", u.arquivo),
                ),
            }
        }
    }
}

// ---------------------------------------------------------------- capturas

const EXT_IMAGEM: &str = "(?:png|jpe?g|webp|gif)";
/// `MOLDE` (`src/lib/cerberus-vigilia.ts:489`): nome com molde (`<pagina>-{antes,depois}.png`) não é arquivo concreto.
static MOLDE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[<>{}*…|]|\.\.\.").expect("regex"));
/// `CAPTURA` (`src/lib/cerberus-vigilia.ts:491`); o `\b` sem a flag `u` é ASCII.
static CAPTURA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)captur|screenshot|print de tela|(?-u:\b)shots?(?-u:\b)").expect("regex")
});
/// `IMAGEM` (`src/lib/cerberus-vigilia.ts:492`).
static IMAGEM: LazyLock<Fancy> = LazyLock::new(|| {
    fancy(&format!(
        r"(?i)(?<![{W}./~-])((?:~|\.{{0,2}}/)?[{W}./~@+-]*[{W}@+-]\.{EXT_IMAGEM})(?![{W}])"
    ))
});
/// `IMAGEM_MOLDE` (`src/lib/cerberus-vigilia.ts:493`).
static IMAGEM_MOLDE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)`([^`\s]*\.{EXT_IMAGEM})`")).expect("regex"));

/// `citacoesDeCaptura` (`src/lib/cerberus-vigilia.ts:496`): `(linha, nome)` dos arquivos de imagem citados em bloco
/// que fala de captura; moldes vêm com nome `None`.
pub fn citacoes_de_captura(texto: &str) -> Vec<(usize, Option<String>)> {
    let mut out = Vec::new();
    for (ini, linhas) in blocos(texto) {
        if !CAPTURA.is_match(&linhas.join(" ")) {
            continue;
        }
        for (k, ln) in linhas.iter().enumerate() {
            let mut vistos: Vec<(usize, usize)> = Vec::new();
            for c in IMAGEM_MOLDE.captures_iter(ln) {
                let m = c.get(1).expect("grupo");
                if MOLDE.is_match(m.as_str()) {
                    vistos.push((m.start(), m.end()));
                    out.push((ini + k, None));
                }
            }
            for (start, nome) in grupos(&IMAGEM, ln) {
                if vistos.iter().any(|&(s, e)| s <= start && start < e) {
                    continue;
                }
                out.push((ini + k, Some(nome)));
            }
        }
    }
    out
}

/// `procurarPorNome` (`src/lib/cerberus-vigilia.ts:519`): busca em largura, sem `.git` nem `node_modules`, até
/// [`BUSCA_MAX`] arquivos.
fn procurar_por_nome(nome: &str, raizes: &[String]) -> Option<String> {
    let mut vistos = 0;
    for raiz in raizes {
        let mut pilha = std::collections::VecDeque::from([raiz.clone()]);
        while let Some(dir) = pilha.pop_front() {
            let Ok(rd) = fs::read_dir(&dir) else {
                continue;
            };
            let entradas: Vec<(String, bool)> = rd
                .filter_map(Result::ok)
                .map(|e| {
                    let eh_dir = e.file_type().is_ok_and(|t| t.is_dir());
                    (e.file_name().to_string_lossy().into_owned(), eh_dir)
                })
                .collect();
            let arquivos: Vec<&String> = entradas
                .iter()
                .filter(|(_, d)| !d)
                .map(|(n, _)| n)
                .collect();
            vistos += arquivos.len();
            if arquivos.iter().any(|n| *n == nome) {
                return Some(js_join(&[&dir, nome]));
            }
            if vistos > BUSCA_MAX {
                return None;
            }
            for (n, d) in &entradas {
                if *d && n != ".git" && n != "node_modules" {
                    pilha.push_back(js_join(&[&dir, n]));
                }
            }
        }
    }
    None
}

enum Resolvida {
    Arquivo(String),
    Ausente,
    SemPasta,
}

/// `resolverCaptura` (`src/lib/cerberus-vigilia.ts:543`).
fn resolver_captura(v: &Vigilia, nome: &str, a: &Agente, dirs_vistos: &[String]) -> Resolvida {
    let p = v.expand_user(nome);
    let wt = a.worktree();
    let rd = v.reports_dir();
    let absoluto = p.starts_with('/');
    let cands: Vec<String> = if absoluto {
        vec![p.clone()]
    } else {
        let mut c = vec![js_join(&[&wt, &p]), js_join(&[&wt, &rd, &p])];
        c.extend(dirs_vistos.iter().map(|d| js_join(&[d, &p])));
        c
    };
    if let Some(c) = cands.iter().find(|c| is_file(c)) {
        return Resolvida::Arquivo(c.clone());
    }
    if nome.trim_matches(['.', '/']).contains('/') {
        return if absoluto || cands.iter().any(|c| is_dir(&js_dirname(c))) {
            Resolvida::Ausente
        } else {
            Resolvida::SemPasta
        };
    }
    let dados = js_join(&[&js_dirname(&wt), "_data", &a.nome]);
    let mut raizes = dirs_vistos.to_vec();
    raizes.extend([js_join(&[&wt, &rd]), dados, wt.clone()]);
    procurar_por_nome(&js_basename(&p, ""), &raizes).map_or(Resolvida::Ausente, Resolvida::Arquivo)
}

/// `gCapturas` (`src/lib/cerberus-vigilia.ts:552`).
pub fn capturas(v: &Vigilia, achados: &mut Achados) {
    for a in alvos_ou_falha(v, achados) {
        let Some(t) = report_de(v, &a, achados) else {
            continue;
        };
        let rp = report_path(v, &a);
        let cits = citacoes_de_captura(&t);
        let moldes = cits.iter().filter(|(_, n)| n.is_none()).count();
        let mut concretas: Vec<(usize, String)> = cits
            .into_iter()
            .filter_map(|(l, n)| n.filter(|n| !n.is_empty()).map(|n| (l, n)))
            .collect();
        if concretas.is_empty() {
            let resto = if moldes > 0 {
                format!(" ({moldes} citações em molde, não conferidas)")
            } else {
                String::new()
            };
            achados.add(
                Passa,
                format!("{}: report não cita captura por nome{resto}", a.nome),
                rp,
            );
            continue;
        }
        let mut dirs_vistos: Vec<String> = Vec::new();
        concretas.sort_by_key(|(_, n)| !n.contains('/'));
        for (ln, nome) in concretas {
            let onde = format!("{rp}:{ln}");
            match resolver_captura(v, &nome, &a, &dirs_vistos) {
                Resolvida::Ausente => {
                    achados.add(Barra, format!("{}: captura citada não existe: {nome}", a.nome), onde)
                }
                Resolvida::SemPasta => achados.add(
                    Aviso,
                    format!(
                        "{}: captura {nome} não resolvida (a pasta dela não existe em nenhum lugar conferido)",
                        a.nome
                    ),
                    onde,
                ),
                Resolvida::Arquivo(p) => {
                    let tam = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    if tam == 0 {
                        achados.add(Barra, format!("{}: captura citada está vazia: {p}", a.nome), onde);
                    } else {
                        let d = js_dirname(&p);
                        if !dirs_vistos.contains(&d) {
                            dirs_vistos.push(d);
                        }
                        achados.add(
                            Passa,
                            format!("{}: captura {nome} existe ({tam} bytes)", a.nome),
                            format!("{p} · {onde}"),
                        );
                    }
                }
            }
        }
        if moldes > 0 {
            achados.add(
                Passa,
                format!(
                    "{}: {moldes} citações em molde (<…>, {{…}}) não conferidas",
                    a.nome
                ),
                rp,
            );
        }
    }
}

// ---------------------------------------------------------------- commits

const SHA: &str = "[0-9a-f]{7,40}";
/// `SHA_CRASE` (`src/lib/cerberus-vigilia.ts:583`).
static SHA_CRASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("`({SHA})`")).expect("regex"));
/// `SHA_SOLTO` (`src/lib/cerberus-vigilia.ts:584`).
static SHA_SOLTO: LazyLock<Fancy> =
    LazyLock::new(|| fancy(&format!(r"(?<![{W}/#-])({SHA})(?![{W}-])")));
/// `FALA_COMMIT` (`src/lib/cerberus-vigilia.ts:585`).
static FALA_COMMIT: LazyLock<Fancy> = LazyLock::new(|| {
    let (b, a) = (before(), after());
    fancy(&format!(
        r"(?i){b}commits?{a}|{b}heads?{a}|{b}base{a}|{b}sha{a}|{b}branch{a}|{b}sobre{a}"
    ))
});
/// `REF` (`src/lib/cerberus-vigilia.ts:586`).
static REF: LazyLock<Fancy> = LazyLock::new(|| {
    fancy(&format!(
        r"(?<![{W}./~-])((?:skill|feat|fix|chore|docs|hades|origin)/[{W}./-]+)"
    ))
});
/// `CAMINHO` (`src/lib/cerberus-vigilia.ts:587`).
static CAMINHO: LazyLock<Fancy> =
    LazyLock::new(|| fancy(&format!(r"(?<![{W}.~/-])(~?/[{W}.@-]+/[{W}./@-]+)")));

/// `pareceSha` (`src/lib/cerberus-vigilia.ts:589`): tem dígito e letra.
fn parece_sha(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_digit()) && s.bytes().any(|b| (b'a'..=b'f').contains(&b))
}

/// `CitacaoDeCommit` (`src/lib/cerberus-vigilia.ts:591`). `na_linha`: a própria linha do hash fala de commit (senão
/// só o bloco fala, e a citação é só avisada).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CitacaoDeCommit {
    pub linha: usize,
    pub sha: String,
    pub bloco: String,
    pub na_linha: bool,
}

/// `citacoesDeCommit` (`src/lib/cerberus-vigilia.ts:601`): hex de 7 a 40 (entre crases ou solto) em bloco que fala de
/// commit, branch ou base, fora de cercas.
pub fn citacoes_de_commit(texto: &str) -> Vec<CitacaoDeCommit> {
    let mut out = Vec::new();
    let mut vistos: Vec<String> = Vec::new();
    for (ini, linhas) in blocos(texto) {
        let bloco = linhas.join(" ");
        if !testa(&FALA_COMMIT, &bloco) {
            continue;
        }
        for (k, ln) in linhas.iter().enumerate() {
            let shas = SHA_CRASE
                .captures_iter(ln)
                .map(|c| c[1].to_owned())
                .chain(grupos(&SHA_SOLTO, ln).into_iter().map(|(_, s)| s));
            for sha in shas {
                if parece_sha(&sha) && !vistos.contains(&sha) {
                    vistos.push(sha.clone());
                    out.push(CitacaoDeCommit {
                        linha: ini + k,
                        sha,
                        bloco: bloco.clone(),
                        na_linha: testa(&FALA_COMMIT, ln),
                    });
                }
            }
        }
    }
    out
}

/// `nivelDoBloco` (`src/lib/cerberus-vigilia.ts:621`): barra quando o bloco fala da branch da unidade (ou de nenhuma
/// outra); aviso quando cita outra ref ou outro repositório.
fn nivel_do_bloco(v: &Vigilia, bloco: &str, branch: &str, worktree: &str) -> (Resultado, String) {
    if !branch.is_empty() && bloco.contains(branch) {
        return (Barra, String::new());
    }
    let mut outras: Vec<String> = Vec::new();
    for (_, r) in grupos(&REF, bloco) {
        if r != branch && !outras.contains(&r) {
            outras.push(r);
        }
    }
    if !outras.is_empty() {
        outras.sort();
        return (Aviso, format!(" (o trecho cita {})", outras.join(", ")));
    }
    let fora = grupos(&CAMINHO, bloco)
        .into_iter()
        .map(|(_, c)| c)
        .find(|c| !real(&v.expand_user(c)).starts_with(worktree));
    match fora {
        Some(c) => (Aviso, format!(" (o trecho cita {c}, fora da worktree)")),
        None => (Barra, String::new()),
    }
}

/// `commitsCitados` (`src/lib/cerberus-vigilia.ts:632`): commits que o report afirma ter feito (hash em linha que
/// fala de commit), para quem precisa conferir só isso.
pub fn commits_citados(report: &str) -> Vec<String> {
    citacoes_de_commit(report)
        .into_iter()
        .filter(|c| c.na_linha)
        .map(|c| c.sha)
        .collect()
}

/// `PROVA_DE_EXECUCAO` (`src/lib/cerberus-vigilia.ts:636`).
static PROVA_DE_EXECUCAO: LazyLock<Fancy> = LazyLock::new(|| {
    let (b, a) = (before(), after());
    fancy(&format!(
        r"(?i){b}(?:test|teste|testes|gate|tsc|check|bun|build|lint|exit|rc){a}"
    ))
});
/// O `0 falhas` que não é marca negativa (`src/lib/cerberus-vigilia.ts:644`); `\d` e `\w` sem a flag `u` são ASCII.
static ZERO_FALHAS: LazyLock<Fancy> =
    LazyLock::new(|| fancy(r"(?i)(?<![0-9.])0\s+(?:fail[A-Za-z0-9_]*|falh[A-Za-z0-9_]*)"));

/// `reportTemProva` (`src/lib/cerberus-vigilia.ts:642`): uma linha com marca positiva (`passou`, `ok`, `rc=0`,
/// `✓`…) e termo de teste ou gate, sem marca negativa. Vale também dentro de cerca de código, onde a saída colada
/// costuma estar.
pub fn report_tem_prova(report: &str) -> bool {
    split_lines(report).iter().any(|ln| {
        let sem = ZERO_FALHAS.replace_all(ln, "");
        testa(&POSITIVO, &sem) && testa(&PROVA_DE_EXECUCAO, &sem) && !testa(&NEGATIVO, &sem)
    })
}

/// `gCommits` (`src/lib/cerberus-vigilia.ts:649`).
pub fn commits(v: &Vigilia, achados: &mut Achados) {
    for a in alvos_ou_falha(v, achados) {
        let Some(t) = report_de(v, &a, achados) else {
            continue;
        };
        let rp = report_path(v, &a);
        let cits = citacoes_de_commit(&t);
        if cits.is_empty() {
            achados.add(Passa, format!("{}: report não cita commit", a.nome), rp);
            continue;
        }
        let br = a.campo("branch");
        let wt = a.worktree();
        let r = (|| -> Result<(), Indisponivel> {
            for c in &cits {
                let onde = format!("{rp}:{} · git -C {wt}", c.linha);
                if !c.na_linha {
                    achados.add(
                        Aviso,
                        format!(
                            "{}: {} em linha que não fala de commit (não conferido)",
                            a.nome, c.sha
                        ),
                        onde,
                    );
                    continue;
                }
                let (rc, tipo) = v.git(&wt, &["cat-file", "-t", &c.sha])?;
                let tipo = js_trim(&tipo);
                if rc != 0 {
                    let (nivel, por) = nivel_do_bloco(v, &c.bloco, &br, &real(&wt));
                    achados.add(
                        nivel,
                        format!(
                            "{}: commit citado {} não existe no repositório{por}",
                            a.nome, c.sha
                        ),
                        onde,
                    );
                    continue;
                }
                if tipo != "commit" {
                    achados.add(
                        Passa,
                        format!("{}: {} é {tipo}, não commit (ignorado)", a.nome, c.sha),
                        onde,
                    );
                    continue;
                }
                if v.git(&wt, &["merge-base", "--is-ancestor", &c.sha, "HEAD"])?
                    .0
                    == 0
                {
                    achados.add(
                        Passa,
                        format!(
                            "{}: commit {} está na branch (ancestral de HEAD)",
                            a.nome, c.sha
                        ),
                        onde,
                    );
                    continue;
                }
                let (nivel, por) = nivel_do_bloco(v, &c.bloco, &br, &real(&wt));
                let qual = if br.is_empty() { "da unidade" } else { &br };
                achados.add(
                    nivel,
                    format!(
                        "{}: commit {} existe mas não está na branch {qual}{por}",
                        a.nome, c.sha
                    ),
                    onde,
                );
            }
            Ok(())
        })();
        tentar(achados, r);
    }
}

// ---------------------------------------------------------------- escopo

/// `EXT` (`src/lib/cerberus-vigilia.ts:690`).
static EXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.[A-Za-z0-9]{1,6}$").expect("regex"));
static ROTULO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:[-*]\s+)?\*\*([^*]+)\*\*").expect("regex"));
static ARQUIVOS_SEUS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)arquivos seus").expect("regex"));
static NAO_TOQUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)não toque|nao toque").expect("regex"));
static CRASES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").expect("regex"));
static MOLDE_ANGULAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("regex"));

/// `paragrafo` (`src/lib/cerberus-vigilia.ts:693`): o parágrafo cujo rótulo em negrito casa com `rotulo` (até a linha
/// em branco).
pub fn paragrafo(texto: &str, rotulo: &Regex) -> Option<String> {
    let linhas = split_lines(texto);
    for (i, l) in linhas.iter().enumerate() {
        let Some(c) = ROTULO.captures(l) else {
            continue;
        };
        if rotulo.is_match(&c[1]) {
            let fim = (i + 1..linhas.len())
                .find(|&j| js_trim(&linhas[j]).is_empty())
                .unwrap_or(linhas.len());
            return Some(linhas[i..fim].join(" "));
        }
    }
    None
}

/// `padroes` (`src/lib/cerberus-vigilia.ts:708`): caminhos citados entre crases; `<x>` vira curinga, `dir/` vira
/// prefixo. Sem barra só entra nome com extensão (casado pelo nome do arquivo), e só se `so_caminho` for falso.
pub fn padroes(par: Option<&str>, so_caminho: bool) -> Vec<String> {
    let mut out = Vec::new();
    for c in CRASES.captures_iter(par.unwrap_or("")) {
        let tok = js_trim(&c[1]);
        if tok.is_empty() || tok.contains(' ') {
            continue;
        }
        if !tok.contains('/') && (so_caminho || !EXT.is_match(tok)) {
            continue;
        }
        out.push(MOLDE_ANGULAR.replace_all(tok, "*").into_owned());
    }
    out
}

/// `casa` (`src/lib/cerberus-vigilia.ts:719`).
pub fn casa(arquivo: &str, pad: &str) -> bool {
    if pad.ends_with('/') {
        return arquivo.starts_with(pad);
    }
    if !pad.contains('/') {
        return fnmatch_case(&js_basename(arquivo, ""), pad);
    }
    fnmatch_case(arquivo, pad)
}

/// `gEscopo` (`src/lib/cerberus-vigilia.ts:725`).
pub fn escopo(v: &Vigilia, achados: &mut Achados) {
    for a in alvos_ou_falha(v, achados) {
        let wt = a.worktree();
        let brief_p = js_join(&[&wt, &v.reports_dir(), "brief.md"]);
        let lido = ler(&brief_p, LEITURA_MAX).and_then(|b| {
            Ok((
                b,
                ler(&report_path(v, &a), LEITURA_MAX)?.unwrap_or_default(),
            ))
        });
        let (brief, report) = match lido {
            Ok(x) => x,
            Err(e) => {
                achados.falta(e);
                continue;
            }
        };
        let Some(brief) = brief else {
            achados.add(
                Aviso,
                format!("{}: brief.md ausente, escopo não conferido", a.nome),
                brief_p,
            );
            continue;
        };
        let seus = padroes(paragrafo(&brief, &ARQUIVOS_SEUS).as_deref(), true);
        let proibidos = padroes(paragrafo(&brief, &NAO_TOQUE).as_deref(), false);
        if seus.is_empty() && proibidos.is_empty() {
            achados.add(
                Passa,
                format!(
                    "{}: brief sem \"Arquivos seus\" nem \"não toque\" entre crases: escopo não declarado",
                    a.nome
                ),
                brief_p,
            );
            continue;
        }
        let base = a.campo("base");
        let r = (|| -> Result<Option<Vec<String>>, Indisponivel> {
            let (rc, mb) = if base.is_empty() {
                (1, String::new())
            } else {
                v.git(&wt, &["merge-base", &base, "HEAD"])?
            };
            if rc != 0 {
                let qual = if base.is_empty() {
                    "(vazia no registro)"
                } else {
                    &base
                };
                achados.add(
                    Aviso,
                    format!(
                        "{}: base {qual} não resolve na worktree; escopo não conferido",
                        a.nome
                    ),
                    format!("git -C {wt} merge-base {base} HEAD"),
                );
                return Ok(None);
            }
            let (rc_diff, saida) = v.git(&wt, &["diff", "--name-only", js_trim(&mb), "HEAD"])?;
            if rc_diff != 0 {
                achados.add(
                    Aviso,
                    format!("{}: git diff falhou (exit {rc_diff})", a.nome),
                    format!("git -C {wt} diff --name-only"),
                );
                return Ok(None);
            }
            Ok(Some(
                split_lines(&saida)
                    .into_iter()
                    .filter(|l| !l.is_empty())
                    .collect(),
            ))
        })();
        let arquivos = match r {
            Ok(Some(a)) => a,
            Ok(None) => continue,
            Err(e) => {
                achados.falta(e);
                continue;
            }
        };
        let onde = format!("git -C {wt} diff --name-only {base}...HEAD · {brief_p}");
        let mut fora = 0;
        for f in &arquivos {
            if !seus.is_empty() && seus.iter().any(|p| casa(f, p)) {
                continue;
            }
            if let Some(hit) = proibidos.iter().find(|p| casa(f, p)) {
                achados.add(
                    Barra,
                    format!(
                        "{}: tocou {f}, que o brief põe em \"não toque\" ({hit})",
                        a.nome
                    ),
                    &onde,
                );
            } else if !seus.is_empty() {
                let citado = report.contains(f.as_str()) || report.contains(&js_basename(f, ""));
                fora += 1;
                let (nivel, nota) = if citado {
                    (Aviso, ", registrado no report")
                } else {
                    (Barra, ", sem registro no report")
                };
                achados.add(
                    nivel,
                    format!("{}: {f} fora de \"Arquivos seus\"{nota}", a.nome),
                    &onde,
                );
            }
        }
        achados.add(
            Passa,
            format!(
                "{}: {} arquivos tocados desde {base}; {} dentro do escopo ({} padrões seus, {} \"não toque\")",
                a.nome,
                arquivos.len(),
                arquivos.len() - fora,
                seus.len(),
                proibidos.len()
            ),
            onde,
        );
    }
}

// ---------------------------------------------------------------- perguntas

static PERGUNTA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^## Q[0-9]+").expect("regex"));

/// `perguntasAbertas` (`src/lib/cerberus-vigilia.ts:779`): títulos `## Qn` sem "(respondida)" nem
/// "## Resposta do dono" depois.
pub fn perguntas_abertas(texto: &str) -> Vec<String> {
    let mut abertas: Vec<String> = Vec::new();
    let mut atual: Option<String> = None;
    for ln in split_lines(texto) {
        if PERGUNTA.is_match(&ln) {
            let titulo = js_trim(ln.get(3..).unwrap_or("")).to_owned();
            if !ln.contains("(respondida)") {
                abertas.push(titulo.clone());
            }
            atual = Some(titulo);
        } else if ln.starts_with("## Resposta do dono")
            && let Some(at) = &atual
            && let Some(i) = abertas.iter().position(|x| x == at)
        {
            abertas.remove(i);
        }
    }
    abertas
}

/// `gPerguntas` (`src/lib/cerberus-vigilia.ts:793`).
pub fn perguntas(v: &Vigilia, achados: &mut Achados) {
    for a in alvos_ou_falha(v, achados) {
        let Some(t) = report_de(v, &a, achados) else {
            continue;
        };
        let qp = js_join(&[&a.worktree(), &v.reports_dir(), "questions.md"]);
        let q = match ler(&qp, LEITURA_MAX) {
            Ok(q) => q,
            Err(e) => {
                achados.falta(e);
                continue;
            }
        };
        let abertas = perguntas_abertas(q.as_deref().unwrap_or(""));
        if !report_done(Some(&t)) {
            achados.add(
                Passa,
                format!(
                    "{}: report não está em DONE; {} perguntas abertas",
                    a.nome,
                    abertas.len()
                ),
                qp,
            );
        } else if !abertas.is_empty() {
            let lista: Vec<String> = abertas.iter().take(5).map(|x| curto(x, 80)).collect();
            achados.add(
                Barra,
                format!(
                    "{}: report em DONE com {} pergunta(s) sem resposta: {}",
                    a.nome,
                    abertas.len(),
                    lista.join("; ")
                ),
                qp,
            );
        } else {
            let sem = if q.is_some() {
                ""
            } else {
                " (sem questions.md)"
            };
            achados.add(
                Passa,
                format!("{}: report em DONE e nenhuma pergunta aberta{sem}", a.nome),
                qp,
            );
        }
    }
}
