use std::fs;
use std::path::Path;

use cerberus_core::modelo::{Cabeca, Modo};
use hades_core::Env;

use super::*;
use crate::vm::Origem;

fn escreve(raiz: &Path, rel: &str, texto: &str) {
    let path = raiz.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, texto).unwrap();
}

fn caixa(raiz: &Path, env: Env) -> Opcoes {
    escreve(
        raiz,
        "skill/kit/config.tsv",
        "HADES_CERBERUS_PORTA\tbarra\nHADES_CERBERUS_LIMIAR\tbarra\nHADES_CERBERUS_VIGILIA\tbarra\n",
    );
    escreve(
        raiz,
        "home/.claude/hades/instance",
        &format!("{}\n", raiz.join("d").display()),
    );
    escreve(raiz, "d/config.env", "HADES_CERBERUS_LIMIAR=avisa\n");
    let env = env
        .with("HOME", raiz.join("home").to_string_lossy())
        .with("HADES_WORK_ROOT", raiz.join("work").to_string_lossy());
    opcoes(env, raiz).com_skill(raiz.join("skill"))
}

#[test]
fn carrega_historico_guardas_e_modos_com_origem() {
    let dir = tempfile::tempdir().unwrap();
    let raiz = fs::canonicalize(dir.path()).unwrap();
    escreve(
        &raiz,
        "skill/loop/cerberus/porta/x.json",
        r#"{"titulo": "X", "origem": "teste", "momentos": ["push"], "tipo": "regra"}"#,
    );
    escreve(
        &raiz,
        "skill/loop/cerberus/limiar/ruim.json",
        r#"{"titulo": "R", "origem": "teste", "momentos": ["push"], "tipo": "magia"}"#,
    );
    escreve(
        &raiz,
        "work/_maquina/cerberus/vereditos.jsonl",
        "{\"ts\":\"2026-11-21T13:02:44.000Z\",\"cabeca\":\"porta\",\"guarda\":\"x\",\"efeito\":\"barra\"}\n",
    );
    let op = caixa(&raiz, Env::empty().with("HADES_CERBERUS_VIGILIA", "off"));

    let d = carregar(op).unwrap();
    assert_eq!(d.historico.len(), 1);
    assert_eq!(d.historico[0]["guarda"], "x");
    assert_eq!(
        d.guardas.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
        ["x"]
    );
    assert_eq!(d.erros.len(), 1);
    assert!(d.erros[0].starts_with("limiar/ruim.json"));
    assert_eq!(
        d.pasta.as_deref(),
        Some(raiz.join("work/_maquina/cerberus").as_path())
    );
    let modos = d.modos.unwrap();
    let resumo: Vec<_> = modos.iter().map(|c| (c.cabeca, c.modo, c.origem)).collect();
    assert_eq!(
        resumo,
        [
            (Cabeca::Porta, Modo::Barra, Origem::Padrao),
            (Cabeca::Limiar, Modo::Avisa, Origem::ConfigEnv),
            (Cabeca::Vigilia, Modo::Off, Origem::Env),
        ]
    );
}

#[test]
fn modo_invalido_vira_recusa_sem_derrubar_o_resto() {
    let dir = tempfile::tempdir().unwrap();
    let raiz = fs::canonicalize(dir.path()).unwrap();
    let op = caixa(&raiz, Env::empty().with("HADES_CERBERUS_PORTA", "talvez"));

    let d = carregar(op).unwrap();
    assert!(d.historico.is_empty());
    assert!(d.guardas.is_empty());
    assert!(d.modos.is_err());
}

fn nova(motivo: &str) -> NovaExcecao {
    NovaExcecao {
        cabeca: "porta".into(),
        guarda: "x".into(),
        momento: "push".into(),
        motivo: motivo.into(),
        alvo: vec!["repo=acme/demo".into()],
        ttl_h: 2,
    }
}

#[test]
fn abre_excecao_com_origem_do_app_e_recusa_motivo_curto() {
    let dir = tempfile::tempdir().unwrap();
    let raiz = fs::canonicalize(dir.path()).unwrap();
    escreve(
        &raiz,
        "skill/loop/cerberus/porta/x.json",
        r#"{"titulo": "X", "origem": "teste", "momentos": ["push"], "tipo": "regra"}"#,
    );
    let env = Env::empty().with("HADES_CERBERUS_VIGILIA", "off");
    let [op_a, op_b, op_c] = [(); 3].map(|_| caixa(&raiz, env.clone()));
    let tsv = fs::read_to_string(raiz.join("skill/kit/config.tsv")).unwrap();
    escreve(
        &raiz,
        "skill/kit/config.tsv",
        &format!("{tsv}HADES_CERBERUS_EXC_TTL_H\t24\n"),
    );
    escreve(&raiz, "d/config.env", "HADES_CERBERUS_EXC_TTL_H=48\n");

    assert!(abrir_excecao(op_a, nova("curto")).is_err());

    let e = abrir_excecao(op_b, nova("falso positivo na fixture")).unwrap();
    assert_eq!(e.texto("origem"), Some("app"));
    assert_eq!(e.texto("guarda"), Some("x"));
    assert_eq!(e.campos().unwrap()["alvo"]["repo"], "acme/demo");

    let d = carregar(op_c).unwrap();
    let lista = d.excecoes.unwrap();
    assert_eq!(lista.len(), 1);
    assert_eq!(lista[0].id(), e.id());
    assert_eq!(d.ttl_h, Ok(48));
}
