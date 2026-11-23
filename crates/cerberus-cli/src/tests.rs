use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use hades_core::Env;
use serde_json::Value;

use super::*;

const CATALOGO: &str = "HADES_CERBERUS_PORTA\tbarra\nHADES_CERBERUS_LIMIAR\tbarra\n\
HADES_CERBERUS_VIGILIA\tavisa\nHADES_CERBERUS_TIMEOUT_S\t60\nHADES_CERBERUS_HIST_KB\t2048\n\
HADES_CERBERUS_EXC_TTL_H\t24\nHADES_JOURNAL\t1\n";

struct Caixa {
    _dir: tempfile::TempDir,
    raiz: PathBuf,
    op: Opcoes,
}

impl Caixa {
    fn nova() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let raiz = fs::canonicalize(dir.path()).unwrap();
        let env = Env::empty()
            .with("HOME", raiz.join("home").to_string_lossy())
            .with("HADES_WORK_ROOT", raiz.join("work").to_string_lossy())
            .with("HADES_CERBERUS_PORTA", "barra")
            .with("HADES_CERBERUS_LIMIAR", "barra")
            .with("HADES_CERBERUS_VIGILIA", "off")
            .with("HADES_JOURNAL", "0");
        let op = Opcoes::novo(env, raiz.clone()).com_skill(raiz.join("skill"));
        let caixa = Self {
            _dir: dir,
            raiz,
            op,
        };
        caixa.escreve("skill/kit/config.tsv", CATALOGO);
        caixa
    }

    fn escreve(&self, rel: &str, texto: &str) -> PathBuf {
        let path = self.raiz.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, texto).unwrap();
        path
    }

    fn exec(&self, cab: &str, id: &str, corpo: &str) {
        let script = self.escreve(
            &format!("skill/loop/cerberus/{cab}/checks/{id}.sh"),
            &format!("#!/bin/bash\n{corpo}\n"),
        );
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        self.escreve(
            &format!("skill/loop/cerberus/{cab}/{id}.json"),
            &format!(
                r#"{{"titulo": "T {id}", "origem": "teste", "momentos": ["release"], "tipo": "exec", "cmd": ["checks/{id}.sh"]}}"#
            ),
        );
    }

    fn emitida(&self) {
        self.escreve(
            "skill/loop/cerberus/porta/so-dono.json",
            r#"{"titulo": "Só o dono", "origem": "teste", "momentos": ["continuo"], "tipo": "emitida"}"#,
        );
    }

    fn roda(&self, args: &[&str]) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let rc = executar(&args, &self.op, &mut out, &mut err);
        (
            rc,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }
}

fn veredito(res: &str) -> String {
    format!(
        r#"echo '{{"resultado": "{res}", "evidencia": [{{"conferido": "conferi {res}", "onde": "t.sh"}}]}}'"#
    )
}

#[test]
fn uso_ajuda_e_rpc() {
    let c = Caixa::nova();
    let (rc, out, _) = c.roda(&[]);
    assert_eq!((rc, out), (2, format!("{USAGE}\n")));
    assert_eq!(c.roda(&["--help"]).0, 0);
    assert_eq!(c.roda(&["-h"]).1, format!("{USAGE}\n"));
    assert_eq!(
        c.roda(&["rpc", "state"]),
        (
            2,
            String::new(),
            "cerberus: operação desconhecida: state\n".into()
        )
    );
    assert_eq!(c.roda(&["rpc"]).2, "cerberus: operação desconhecida: \n");
}

#[test]
fn recusas_do_parser_e_do_despacho() {
    let c = Caixa::nova();
    assert_eq!(
        c.roda(&["check"]),
        (
            2,
            String::new(),
            "cerberus: comando desconhecido: check\n".into()
        )
    );
    assert_eq!(
        c.roda(&["avaliar", "--dir"]).2,
        "cerberus: --dir pede um valor\n"
    );
    assert_eq!(c.roda(&["avaliar"]).2, "cerberus: avaliar <momento>\n");
    assert_eq!(
        c.roda(&["avaliar", "nunca"]).2,
        format!(
            "cerberus: momento inválido: nunca ({})\n",
            cerberus_core::modelo::Momento::lista()
        )
    );
    assert_eq!(c.roda(&["avaliar", "release", "--ctx", "x"]).0, 2);
    assert_eq!(
        c.roda(&["registrar", "--cabeca", "porta"]).2,
        "cerberus: registrar pede --guarda\n"
    );
    assert_eq!(
        c.roda(&[
            "registrar",
            "--cabeca",
            "porta",
            "--guarda",
            "g",
            "--momento",
            "m",
            "--resultado",
            "",
            "--json"
        ])
        .2,
        "cerberus: registrar pede --resultado\n"
    );
    assert_eq!(
        c.roda(&[
            "registrar",
            "--cabeca",
            "porta",
            "--guarda",
            "g",
            "--momento",
            "m",
            "--resultado",
            "ok",
            "--evidencia",
            "sem barra"
        ])
        .2,
        "cerberus: --evidencia 'sem barra': \"<conferido>|<onde>\"\n"
    );
    for l in ["0", "00", "-1", "x", "", "1.5"] {
        assert_eq!(
            c.roda(&["historico", "--limite", l]).2,
            "cerberus: --limite: inteiro positivo\n",
            "{l}"
        );
    }
    assert_eq!(c.roda(&["historico", "--limite", "007"]).0, 0);
    let longa = "cerberus: excecao listar [--json] | excecao abrir --cabeca <c> --guarda <id> --momento <m> --motivo \"<t>\" [--alvo k=v]\n";
    assert_eq!(c.roda(&["excecao"]).2, longa);
    assert_eq!(
        c.roda(&["excecao", "abrir", "--cabeca", "porta"]).2,
        "cerberus: excecao abrir pede --guarda\n"
    );
}

#[test]
fn avaliar_registrar_e_historico() {
    let c = Caixa::nova();
    c.exec("porta", "t-o", &veredito("ok"));
    c.exec("limiar", "t-b", &veredito("barra"));
    c.emitida();

    let (rc, out, err) = c.roda(&["guardas"]);
    assert_eq!((rc, err.as_str()), (0, ""));
    assert!(out.contains(&format!(
        "{} {} {} {} T t-o\n",
        pad("porta", 8),
        pad("t-o", 28),
        pad("exec", 8),
        pad("release", 20)
    )));

    let (rc, out, _) = c.roda(&["avaliar", "release"]);
    assert_eq!(rc, 1);
    let linhas: Vec<&str> = out.lines().collect();
    assert_eq!(linhas.len(), 3, "{out}");
    assert!(
        linhas.contains(&"  ✗ barra    limiar/t-b: conferi barra"),
        "{out}"
    );
    assert!(
        linhas.contains(&"  ✓ ok       porta/t-o: conferi ok"),
        "{out}"
    );
    assert_eq!(
        linhas[2],
        "cerberus release: barra — 2 veredito(s), 1 barra(m), 0 aviso(s) (cabeça em off: vigilia)"
    );
    let (rc, out, _) = c.roda(&["avaliar", "release", "--cabeca", "porta", "--json"]);
    assert_eq!(rc, 0);
    let v: Value = serde_json::from_str(&out).unwrap();
    let chaves: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        chaves,
        [
            "avaliacao",
            "momento",
            "resultado",
            "vereditos",
            "barras",
            "avisos",
            "ignoradas"
        ]
    );

    let (rc, out, _) = c.roda(&["avaliar", "continuo", "--cabeca", "porta"]);
    assert_eq!((rc, out.as_str()), (0, ""));

    let (rc, _, err) = c.roda(&[
        "registrar",
        "--cabeca",
        "porta",
        "--guarda",
        "so-dono",
        "--momento",
        "continuo",
        "--resultado",
        "aviso",
        "--evidencia",
        "olhei | aqui|ali ",
        "--detalhe",
        "det",
    ]);
    assert_eq!((rc, err.as_str()), (0, ""));
    let (_, out, _) = c.roda(&["historico", "--cabeca", "porta", "--limite", "1", "--json"]);
    let rows: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        rows[0]["evidencia"][0],
        serde_json::json!({"conferido": "olhei | aqui", "onde": "ali"})
    );
    assert_eq!(rows[0]["dir"], c.raiz.to_string_lossy().as_ref());
    let (_, out, _) = c.roda(&["historico", "--efeito", "aviso"]);
    assert!(
        out.ends_with(" aviso    porta/so-dono @continuo: det\n"),
        "{out}"
    );
}

#[test]
fn modos_excecao_e_estado() {
    let c = Caixa::nova();
    c.exec("porta", "t-b", &veredito("barra"));
    let (rc, out, _) = c.roda(&["modos"]);
    assert_eq!(
        (rc, out.as_str()),
        (0, "porta    barra\nlimiar   barra\nvigilia  off\n")
    );
    assert_eq!(
        c.roda(&["modos", "--json"]).1,
        "{\"porta\": \"barra\", \"limiar\": \"barra\", \"vigilia\": \"off\"}\n"
    );

    let (rc, out, err) = c.roda(&[
        "excecao",
        "abrir",
        "--cabeca",
        "porta",
        "--guarda",
        "t-b",
        "--momento",
        "release",
        "--motivo",
        "teste de exceção",
        "--alvo",
        "pr=1234",
    ]);
    assert_eq!((rc, err.as_str()), (0, ""));
    assert!(
        out.starts_with("exceção ") && out.contains(" aberta: porta/t-b @release, uso único, até "),
        "{out}"
    );
    let (_, out, _) = c.roda(&["excecao", "listar"]);
    assert!(
        out.contains(&format!(
            " {} porta/t-b @release {{'pr': '1234'}}: teste de exceção\n",
            pad("aberta", 20)
        )),
        "{out}"
    );

    assert_eq!(c.roda(&["avaliar", "release", "--ctx", "pr=1234"]).0, 0);
    let (_, out, _) = c.roda(&["excecao", "listar"]);
    assert!(out.contains(" usada em "), "{out}");
    assert_eq!(c.roda(&["avaliar", "release", "--ctx", "pr=1234"]).0, 1);

    let (rc, out, _) = c.roda(&["estado"]);
    assert_eq!(rc, 0);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["barrados"][0]["vezes"], 1);
}

#[test]
fn check_sem_guarda_portada_responde_o_uso() {
    let ce = CheckEnv::default();
    for args in [
        &[][..],
        &["limiar/x"][..],
        &["porta/fantasma"][..],
        &["vigilia/coerencia", "registro"][..],
    ] {
        let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(check(&args, &ce, &mut out, &mut err), 2, "{args:?}");
        assert!(out.is_empty());
        assert!(String::from_utf8_lossy(&err).contains(USO_CHECK));
    }
}

#[test]
fn check_da_auditoria_sem_guarda_responde_o_uso_dela() {
    let ce = CheckEnv::default();
    for args in [
        &["vigilia/auditoria"][..],
        &["vigilia/auditoria", "nada"][..],
        &["vigilia/auditoria", "status", "x"][..],
    ] {
        let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(check(&args, &ce, &mut out, &mut err), 2, "{args:?}");
        assert!(out.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&err).trim_end(),
            "uso: auditoria.py status|gates|capturas|commits|escopo|perguntas"
        );
    }
}

#[test]
fn check_da_porta_imprime_o_veredito_numa_linha() {
    let ce = CheckEnv::default();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        check(&["porta/tunel-acesso".to_string()], &ce, &mut out, &mut err),
        0
    );
    let texto = String::from_utf8(out).unwrap();
    assert_eq!(texto.lines().count(), 1);
    let v: Value = serde_json::from_str(texto.trim_end()).unwrap();
    assert_eq!(v["resultado"], "barra");
}
