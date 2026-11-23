//! Casos de `loop/test/ts/cerberus-vigilia-legado.test.ts` (alvos e auditoria), com dados fictícios.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::json;

use super::auditoria::*;
use super::*;

fn git(wt: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(wt)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("git");
    String::from_utf8_lossy(&o.stdout).trim().to_owned()
}

struct Caixa {
    _tmp: tempfile::TempDir,
    raiz: std::path::PathBuf,
    n: usize,
}

impl Caixa {
    fn nova() -> Self {
        let tmp = tempfile::tempdir().expect("tmp");
        let raiz = tmp.path().to_path_buf();
        fs::create_dir_all(raiz.join("work/acme__demo/orchestration/agents")).unwrap();
        fs::create_dir_all(raiz.join("work/acme__demo/locks")).unwrap();
        Self {
            _tmp: tmp,
            raiz,
            n: 0,
        }
    }

    fn repo(&mut self) -> std::path::PathBuf {
        self.n += 1;
        let wt = self.raiz.join(format!("wt{}", self.n));
        fs::create_dir_all(&wt).unwrap();
        git(&wt, &["init", "-q", "-b", "feature"]);
        git(
            &wt,
            &["remote", "add", "origin", "git@github.com:acme/demo.git"],
        );
        fs::write(wt.join("a.txt"), "a").unwrap();
        git(&wt, &["add", "a.txt"]);
        git(&wt, &["commit", "-qm", "inicio"]);
        wt
    }

    /// Agente no registro, com a worktree e os arquivos de `.design-sync-reports`.
    fn agente(
        &mut self,
        report: Option<&str>,
        extra: &[(&str, &str)],
    ) -> (String, std::path::PathBuf) {
        let wt = self.repo();
        let nome = format!("ag{}", self.n);
        let rep = wt.join(".design-sync-reports");
        fs::create_dir_all(&rep).unwrap();
        if let Some(r) = report {
            fs::write(rep.join("report.md"), r).unwrap();
        }
        let base = git(&wt, &["rev-parse", "HEAD"]);
        let mut reg = json!({"name": nome, "worktree": wt.to_string_lossy(), "branch": "feature", "base": base,
            "mode": "impl", "state": "active", "pr": "1234", "createdAt": "2000-01-01T00:00:00Z"});
        for (k, v) in extra {
            reg[*k] = json!(v);
        }
        let f = self
            .raiz
            .join(format!("work/acme__demo/orchestration/agents/{nome}.json"));
        fs::write(f, reg.to_string()).unwrap();
        (nome, wt)
    }

    fn ce(&self, dir: &Path, momento: &str, ctx: &str) -> CheckEnv {
        let mut env: BTreeMap<String, String> = std::env::vars().collect();
        env.insert(
            "HADES_WORK_ROOT".into(),
            self.raiz.join("work").to_string_lossy().into(),
        );
        env.insert(
            "HADES_HOME".into(),
            self.raiz.join("hh").to_string_lossy().into(),
        );
        env.insert(
            "CERBERUS_SKILL".into(),
            self.raiz.join("skill").to_string_lossy().into(),
        );
        env.insert("CERBERUS_MOMENTO".into(), momento.into());
        env.insert("CERBERUS_CTX".into(), ctx.into());
        env.insert("CERBERUS_DIR".into(), dir.to_string_lossy().into());
        CheckEnv::do_ambiente(env, dir)
    }

    fn roda(&self, guarda: &str, wt: &Path, nome: &str) -> CheckVerdict {
        let ce = self.ce(wt, "done", &format!(r#"{{"agente": "{nome}"}}"#));
        rodar_vigilia(&AUDITORIA, guarda, &ce).expect("guarda")
    }
}

fn res(v: &CheckVerdict) -> &'static str {
    v.resultado.as_str()
}

#[test]
fn blocos_e_report_done() {
    let t = "# T\npar\ncontinua\n\n- item\n  sub\n```\ndentro\n```\n| a |\n1. um\n";
    let b = blocos(t);
    let linhas: Vec<usize> = b.iter().map(|(i, _)| *i).collect();
    assert_eq!(linhas, [1, 2, 5, 10, 11]);
    assert_eq!(b[1].1, ["par", "continua"]);
    assert!(report_done(Some("STATUS: DONE\n")));
    assert!(report_done(Some("  STATUS: DONE — ok")));
    assert!(!report_done(Some("")));
    assert!(!report_done(Some("x\nSTATUS: DONE")));
    assert!(!report_done(None));
}

#[test]
fn curto_e_emitir() {
    assert_eq!(curto("a  b\n c", 500), "a b c");
    assert_eq!(curto("abcdef", 4), "abc…");
    let mut a = Achados::default();
    let v = emitir(&a, "tudo certo");
    assert_eq!(v.evidencia[0].conferido, "nada a conferir");
    assert_eq!(v.detalhe.as_deref(), Some("tudo certo"));
    a.add(Resultado::Ok, "bom", "x");
    a.add(Resultado::Aviso, "meio", "y");
    a.add(Resultado::Barra, "ruim", "z");
    let v = emitir(&a, "ok");
    assert_eq!(res(&v), "barra");
    assert_eq!(v.evidencia[0].conferido, "[barra] ruim");
    assert_eq!(v.detalhe.as_deref(), Some("ruim"));
    for i in 0..25 {
        a.add(Resultado::Ok, i.to_string(), "x");
    }
    let v = emitir(&a, "ok");
    assert_eq!(v.evidencia.len(), 20);
    assert_eq!(
        v.evidencia[19].conferido,
        "mais 9 itens conferidos, omitidos"
    );
}

#[test]
fn caminhos_como_o_node() {
    assert_eq!(js_join(&["/wt", "/abs", "r.md"]), "/wt/abs/r.md");
    assert_eq!(js_join(&["", ".x", "r.md"]), ".x/r.md");
    assert_eq!(js_join(&["/a/b", "../c"]), "/a/c");
    assert_eq!(js_dirname("/a/b/"), "/a");
    assert_eq!(js_dirname("a"), ".");
    assert_eq!(js_dirname("/a"), "/");
    assert_eq!(js_basename("/x/ag1.json", ".json"), "ag1");
    assert_eq!(py_quote("a'b"), "\"a'b\"");
    assert_eq!(py_quote("../x"), "'../x'");
}

#[test]
fn alegacoes_e_prova() {
    let t = "STATUS: DONE\n\n- pela fila: `run test --label demo-1` passou\n\n## Gate (fila)\n\nlint ok\n\n- pela fila: run build falhou\n";
    let a = alegacoes_de_gate(t);
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].kind.as_deref(), Some("test"));
    assert_eq!(a[0].label.as_deref(), Some("demo-1"));
    assert_eq!(a[1].linha, 5);
    assert!(alegacoes_de_gate("```\npela fila: run test passou\n```\n").is_empty());
    assert!(report_tem_prova("test: 12 passed, 0 failed"));
    assert!(!report_tem_prova("Tests: 12 passed"));
    assert!(!report_tem_prova("test failed"));
    assert!(report_tem_prova("rc=0 no gate"));
    assert!(!report_tem_prova("nada aqui"));
}

#[test]
fn citacoes_de_commit_e_captura() {
    let t = "- commit `abc1234def` na branch feature\n  e 1234abcd solto\n\nversão abc1234def\n\n- commit deadbeef\n";
    let c = citacoes_de_commit(t);
    assert_eq!(c.len(), 2);
    assert!(c[0].na_linha && !c[1].na_linha);
    assert_eq!(commits_citados(t), ["abc1234def"]);
    let cap = citacoes_de_captura(
        "- capturas `<pagina>-{antes,depois}.png` e shots/a.png\n\nsem.png aqui\n",
    );
    assert_eq!(cap, [(1, None), (1, Some("shots/a.png".to_owned()))]);
}

#[test]
fn escopo_puro() {
    let brief = "## Item\n\n**Arquivos seus (e só estes):** `src/`, `docs/*.md`, `kit/<x>.sh`\n\n**Em voo, não toque:** `lib.sh`, `loop/bin/`\n";
    let seus = padroes(
        paragrafo(brief, &Regex::new("(?i)arquivos seus").unwrap()).as_deref(),
        true,
    );
    assert_eq!(seus, ["src/", "docs/*.md", "kit/*.sh"]);
    let nao = padroes(
        paragrafo(brief, &Regex::new("(?i)não toque").unwrap()).as_deref(),
        false,
    );
    assert_eq!(nao, ["lib.sh", "loop/bin/"]);
    assert!(casa("src/x.rs", "src/"));
    assert!(casa("a/lib.sh", "lib.sh"));
    assert!(casa("kit/a.sh", "kit/*.sh"));
    assert!(!casa("docs/x.txt", "docs/*.md"));
}

#[test]
fn perguntas_abertas_como_o_lib_sh() {
    let q = "## Q1 — x\n\n## Q2 — y (respondida)\n\n## Q3 — z\n\n## Resposta do dono\nsim\n";
    assert_eq!(perguntas_abertas(q), ["Q1 — x"]);
}

#[test]
fn alvos_avisam_sem_contexto() {
    let mut c = Caixa::nova();
    let wt = c.repo();
    let sem = rodar_vigilia(&AUDITORIA, "status", &c.ce(&wt, "done", "{}")).unwrap();
    assert_eq!(res(&sem), "aviso");
    let fora = rodar_vigilia(
        &AUDITORIA,
        "status",
        &c.ce(&wt, "done", r#"{"agente": "../x"}"#),
    )
    .unwrap();
    assert_eq!(
        fora.evidencia[0].conferido,
        "[aviso] não conferido: agente fora do padrão no contexto: '../x'"
    );
    let fantasma = rodar_vigilia(
        &AUDITORIA,
        "status",
        &c.ce(&wt, "done", r#"{"agente": "fantasma"}"#),
    )
    .unwrap();
    assert_eq!(res(&fantasma), "aviso");
    assert!(rodar_vigilia(&AUDITORIA, "nada", &c.ce(&wt, "done", "{}")).is_none());
}

#[test]
fn alvos_da_consolidacao() {
    let mut c = Caixa::nova();
    let (_, wt) = c.agente(Some("STATUS: DONE\n"), &[]);
    c.agente(
        Some("STATUS: PARKED\n"),
        &[("pr", "99"), ("state", "archived")],
    );
    let todos = rodar_vigilia(
        &AUDITORIA,
        "status",
        &c.ce(&wt, "release", r#"{"branch": "feat/x"}"#),
    )
    .unwrap();
    assert_eq!(res(&todos), "ok");
    let cons = c.raiz.join("work/acme__demo/orchestration/consolidations");
    fs::create_dir_all(&cons).unwrap();
    fs::write(cons.join("feat_x.json"), r#"{"members": [1234, 99]}"#).unwrap();
    let m = rodar_vigilia(
        &AUDITORIA,
        "status",
        &c.ce(&wt, "release", r#"{"branch": "feat/x"}"#),
    )
    .unwrap();
    assert_eq!(res(&m), "barra");
    assert!(
        m.evidencia
            .iter()
            .any(|e| e.conferido.contains("(PRs 1234, 99): 2 agentes"))
    );
    fs::write(cons.join("feat_x.json"), "{x").unwrap();
    let ruim = rodar_vigilia(
        &AUDITORIA,
        "status",
        &c.ce(&wt, "release", r#"{"branch": "feat/x"}"#),
    )
    .unwrap();
    assert_eq!(res(&ruim), "aviso");
}

#[test]
fn auditoria_status() {
    let mut c = Caixa::nova();
    let (n, wt) = c.agente(Some("STATUS: DONE\n"), &[]);
    assert_eq!(res(&c.roda("status", &wt, &n)), "ok");
    let (n, wt) = c.agente(Some("STATUS: PARKED — teto\n"), &[]);
    assert_eq!(res(&c.roda("status", &wt, &n)), "barra");
    let (n, wt) = c.agente(Some(""), &[]);
    assert!(
        c.roda("status", &wt, &n).evidencia[0]
            .conferido
            .contains("\"(vazio)\"")
    );
    let (n, wt) = c.agente(None, &[]);
    assert_eq!(
        c.roda("status", &wt, &n).detalhe.as_deref(),
        Some(format!("{n}: report.md ausente").as_str())
    );
}

#[test]
fn auditoria_gates() {
    let mut c = Caixa::nova();
    let r = "STATUS: DONE\n\n- pela fila: run test --label demo-1 passou\n";
    let (n, wt) = c.agente(Some(r), &[]);
    assert_eq!(res(&c.roda("gates", &wt, &n)), "barra");
    let j = c.raiz.join("work/acme__demo/locks/gate-journal.tsv");
    let linha = |ts: &str, kind: &str, rc: i32| {
        format!(
            "{ts}\tdone\t{kind}\tdemo-1\tx\t{}\trc={rc}\n",
            wt.to_string_lossy()
        )
    };
    fs::write(&j, linha("2026-01-01T00:00:00Z", "test:integration", 0)).unwrap();
    assert_eq!(res(&c.roda("gates", &wt, &n)), "ok");
    fs::write(
        &j,
        linha("2026-01-01T00:00:00Z", "test", 0) + &linha("2026-01-02T00:00:00Z", "test", 1),
    )
    .unwrap();
    assert_eq!(res(&c.roda("gates", &wt, &n)), "barra");
    fs::write(&j, linha("1999-01-01T00:00:00Z", "test", 0)).unwrap();
    assert_eq!(res(&c.roda("gates", &wt, &n)), "barra");
}

#[test]
fn auditoria_capturas() {
    let mut c = Caixa::nova();
    let (n, wt) = c.agente(Some("STATUS: DONE\n\n- captura `sumiu.png`\n"), &[]);
    assert_eq!(res(&c.roda("capturas", &wt, &n)), "barra");
    fs::create_dir_all(wt.join("docs/shots")).unwrap();
    fs::write(wt.join("docs/shots/sumiu.png"), "png").unwrap();
    assert_eq!(res(&c.roda("capturas", &wt, &n)), "ok");
    fs::write(wt.join("docs/shots/sumiu.png"), "").unwrap();
    assert_eq!(res(&c.roda("capturas", &wt, &n)), "barra");
    let (n, wt) = c.agente(Some("STATUS: DONE\n\n- captura nada/aqui/x.png\n"), &[]);
    assert_eq!(res(&c.roda("capturas", &wt, &n)), "aviso");
}

#[test]
fn auditoria_commits() {
    let mut c = Caixa::nova();
    let (n, wt) = c.agente(
        Some("STATUS: DONE\n\n- commit `abc1234def` na branch feature\n"),
        &[],
    );
    assert_eq!(res(&c.roda("commits", &wt, &n)), "barra");
    let (n, wt) = c.agente(
        Some("STATUS: DONE\n\n- commit `abc1234def` de skill/outra\n"),
        &[],
    );
    assert_eq!(res(&c.roda("commits", &wt, &n)), "aviso");
    let (n, wt) = c.agente(
        Some("STATUS: DONE\n\n- commit `abc1234def` em /opt/outro/repo\n"),
        &[],
    );
    assert_eq!(res(&c.roda("commits", &wt, &n)), "aviso");
    let (n, wt) = c.agente(None, &[]);
    let sha = git(&wt, &["rev-parse", "HEAD"]);
    let rep = wt.join(".design-sync-reports/report.md");
    fs::write(&rep, format!("STATUS: DONE\n\n- commit `{}`\n", &sha[..10])).unwrap();
    let v = c.roda("commits", &wt, &n);
    if parece(&sha[..10]) {
        assert_eq!(res(&v), "ok");
    }
}

fn parece(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_digit()) && s.bytes().any(|b| (b'a'..=b'f').contains(&b))
}

#[test]
fn auditoria_escopo() {
    let brief = "**Arquivos seus:** `src/`\n\n**Em voo, não toque:** `lib.sh`\n";
    let mut c = Caixa::nova();
    let (n, wt) = c.agente(Some("STATUS: DONE\n"), &[]);
    assert_eq!(res(&c.roda("escopo", &wt, &n)), "aviso");
    fs::write(wt.join(".design-sync-reports/brief.md"), brief).unwrap();
    fs::create_dir_all(wt.join("src")).unwrap();
    fs::write(wt.join("src/x.rs"), "x").unwrap();
    git(&wt, &["add", "src"]);
    git(&wt, &["commit", "-qm", "muda"]);
    assert_eq!(res(&c.roda("escopo", &wt, &n)), "ok");
    fs::write(wt.join("lib.sh"), "x").unwrap();
    git(&wt, &["add", "lib.sh"]);
    git(&wt, &["commit", "-qm", "muda"]);
    assert_eq!(res(&c.roda("escopo", &wt, &n)), "barra");
    let (n, wt) = c.agente(Some("STATUS: DONE\n"), &[("base", "naoexiste")]);
    fs::write(wt.join(".design-sync-reports/brief.md"), brief).unwrap();
    assert_eq!(res(&c.roda("escopo", &wt, &n)), "aviso");
}

#[test]
fn auditoria_perguntas() {
    let mut c = Caixa::nova();
    let (n, wt) = c.agente(Some("STATUS: DONE\n"), &[]);
    assert_eq!(res(&c.roda("perguntas", &wt, &n)), "ok");
    fs::write(
        wt.join(".design-sync-reports/questions.md"),
        "## Q1 — x\n\nctx\n",
    )
    .unwrap();
    assert_eq!(res(&c.roda("perguntas", &wt, &n)), "barra");
    let (n, wt) = c.agente(Some("STATUS: ACTIVE\n"), &[]);
    fs::write(wt.join(".design-sync-reports/questions.md"), "## Q1 — x\n").unwrap();
    assert_eq!(res(&c.roda("perguntas", &wt, &n)), "ok");
}
