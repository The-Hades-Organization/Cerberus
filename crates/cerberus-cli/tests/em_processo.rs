//! O [`ExecutorEmProcesso`] contra o subprocesso: o mesmo veredito numa guarda da Porta, com o binário desta crate
//! pelo caminho (`bin_check`), nunca pelo `PATH`; e o que não é guarda portada continua indo para o subprocesso.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cerberus_core::config::Cfg;
use cerberus_core::{Cerberus, ExecutorEmProcesso, ExecutorProcesso, Opcoes};
use hades_core::Env;

fn cfg() -> Cfg {
    [
        ("HADES_CERBERUS_PORTA", "barra"),
        ("HADES_CERBERUS_LIMIAR", "barra"),
        ("HADES_CERBERUS_VIGILIA", "barra"),
        ("HADES_CERBERUS_TIMEOUT_S", "30"),
        ("HADES_CERBERUS_HIST_KB", "1024"),
        ("HADES_CERBERUS_EXC_TTL_H", "24"),
        ("HADES_JOURNAL", "0"),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .collect()
}

fn escreve(raiz: &Path, rel: &str, texto: &str) -> PathBuf {
    let path = raiz.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, texto).unwrap();
    path
}

fn caixa(raiz: &Path) -> Opcoes {
    escreve(
        raiz,
        "skill/loop/cerberus/porta/tunel.json",
        r#"{"titulo": "T", "origem": "o", "momentos": ["push"], "tipo": "exec", "cmd": ["hades", "cerberus-check", "porta/tunel-acesso"]}"#,
    );
    let script = escreve(
        raiz,
        "skill/loop/cerberus/porta/checks/local.sh",
        "#!/bin/bash\necho '{\"resultado\": \"aviso\", \"evidencia\": [{\"conferido\": \"script\", \"onde\": \"local.sh\"}]}'\n",
    );
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    escreve(
        raiz,
        "skill/loop/cerberus/porta/local.json",
        r#"{"titulo": "L", "origem": "o", "momentos": ["push"], "tipo": "exec", "cmd": ["checks/local.sh"]}"#,
    );
    escreve(
        raiz,
        "skill/loop/cerberus/vigilia/so-dono.json",
        r#"{"titulo": "Só o dono", "origem": "teste", "momentos": ["continuo"], "tipo": "emitida"}"#,
    );
    let env = Env::empty()
        .with("HOME", raiz.join("home").to_string_lossy())
        .with("HADES_WORK_ROOT", raiz.join("work").to_string_lossy());
    Opcoes::novo(env, raiz.to_path_buf()).com_skill(raiz.join("skill"))
}

#[test]
fn em_processo_da_o_veredito_do_subprocesso() {
    let dir = tempfile::tempdir().unwrap();
    let raiz = fs::canonicalize(dir.path()).unwrap();
    let op = caixa(&raiz).com_bin_check(PathBuf::from(env!("CARGO_BIN_EXE_cerberus")));
    let ctx = Default::default();
    let sub = Cerberus::abrir(op.clone()).com_executor(Arc::new(ExecutorProcesso));
    let em = Cerberus::abrir(op).com_executor(Arc::new(ExecutorEmProcesso::novo()));
    for id in ["tunel", "local"] {
        let a = sub.ensaiar("porta", id, "push", &ctx, Some(cfg())).unwrap();
        let b = em.ensaiar("porta", id, "push", &ctx, Some(cfg())).unwrap();
        assert_eq!(a, b, "{id}");
    }
    let t = em
        .ensaiar("porta", "tunel", "push", &ctx, Some(cfg()))
        .unwrap();
    assert_eq!(t.0["resultado"], "barra");
    let l = em
        .ensaiar("porta", "local", "push", &ctx, Some(cfg()))
        .unwrap();
    assert_eq!(l.0["evidencia"][0]["conferido"], "script");
}

#[test]
fn em_processo_nao_precisa_de_binario() {
    let dir = tempfile::tempdir().unwrap();
    let raiz = fs::canonicalize(dir.path()).unwrap();
    let op = caixa(&raiz).com_bin_check(raiz.join("nao-existe/cerberus"));
    let ctx = Default::default();
    let sub = Cerberus::abrir(op.clone()).com_executor(Arc::new(ExecutorProcesso));
    let em = Cerberus::abrir(op).com_executor(Arc::new(ExecutorEmProcesso::novo()));
    let a = sub
        .ensaiar("porta", "tunel", "push", &ctx, Some(cfg()))
        .unwrap();
    assert_eq!(a.0["invalido"], true);
    let b = em
        .ensaiar("porta", "tunel", "push", &ctx, Some(cfg()))
        .unwrap();
    assert_eq!(b.0["resultado"], "barra");
    assert!(b.0.get("invalido").is_none());
}
