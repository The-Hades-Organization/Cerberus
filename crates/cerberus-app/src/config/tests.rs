use super::*;
use std::fs;
use tempfile::TempDir;

const KIT: &str = "\
HADES_CERBERUS_PORTA\tavisa\tenum:off,avisa,barra\tCerberus\tModo da Porta\tnext\tModo.\n\
HADES_CERBERUS_TIMEOUT_S\t30\tint:1:600\tCerberus\tTempo das guardas\tnext\tTempo.\n\
HADES_CERBERUS_PORTA_TUNEL_HOSTS\t\tglobs\tCerberus\tHosts do túnel\tnext\tHosts.\n\
HADES_OUTRA\t1\tbool\tG\tOutra\tnext\tNão é do Cerberus.\n";

struct Sandbox {
    _dir: TempDir,
    env: Env,
    cwd: std::path::PathBuf,
}

fn sandbox() -> Sandbox {
    let dir = TempDir::new().unwrap();
    let skill = dir.path().join("skill");
    let kit = skill.join("kit");
    let dados = dir.path().join("dados");
    fs::create_dir_all(&kit).unwrap();
    fs::create_dir_all(&dados).unwrap();
    fs::write(kit.join("config.tsv"), KIT).unwrap();
    let hh = dir.path().join(".claude/hades");
    fs::create_dir_all(&hh).unwrap();
    fs::write(hh.join("instance"), format!("{}\n", dados.display())).unwrap();
    let env = Env::empty()
        .with("HOME", dir.path().to_string_lossy())
        .with("HADES_SKILL_DIR", skill.to_string_lossy())
        .with("HADES_DATA", dados.to_string_lossy());
    Sandbox {
        cwd: dir.path().to_path_buf(),
        env,
        _dir: dir,
    }
}

fn textos(linhas: &[Linha]) -> Vec<String> {
    linhas.iter().map(|l| l.valor.clone()).collect()
}

fn valor(s: &Sandbox, chave: &str) -> String {
    config::config_value(&s.env, &s.cwd, chave, "?")
}

#[test]
fn mostra_so_as_chaves_do_cerberus_com_tipo() {
    let s = sandbox();
    let linhas = ConfigLocal::new(&s.env).carregar().unwrap();
    let chaves: Vec<_> = linhas.iter().map(|l| l.chave.as_str()).collect();
    assert_eq!(
        chaves,
        [
            "HADES_CERBERUS_PORTA",
            "HADES_CERBERUS_TIMEOUT_S",
            "HADES_CERBERUS_PORTA_TUNEL_HOSTS"
        ]
    );
    assert_eq!(
        linhas[0].opcoes(),
        Some(vec!["off".into(), "avisa".into(), "barra".into()])
    );
    assert_eq!(linhas[1].faixa(), Some((1, 600)));
    assert_eq!(linhas[0].origem, Origem::Padrao);
}

#[test]
fn gravar_pela_pagina_e_ler_pelo_config_value_do_cli() {
    let s = sandbox();
    let fonte = ConfigLocal::new(&s.env);
    let linhas = fonte.carregar().unwrap();
    let mut t = textos(&linhas);
    t[0] = " barra ".into();
    t[1] = "045".into();
    t[2] = " *.demo.test , acme-? ".into();
    assert_eq!(pendentes(&linhas, &t), 3);
    assert_eq!(salvar(&fonte, &linhas, &t), Ok(3));
    assert_eq!(valor(&s, "HADES_CERBERUS_PORTA"), "barra");
    assert_eq!(valor(&s, "HADES_CERBERUS_TIMEOUT_S"), "45");
    assert_eq!(
        valor(&s, "HADES_CERBERUS_PORTA_TUNEL_HOSTS"),
        "*.demo.test,acme-?"
    );
    let depois = fonte.carregar().unwrap();
    assert_eq!(depois[0].origem, Origem::ConfigEnv);

    let mut t = textos(&depois);
    t[2] = "  ".into();
    assert_eq!(erros(&depois, &t), vec![None, None, None]);
    assert_eq!(salvar(&fonte, &depois, &t), Ok(1));
    assert_eq!(valor(&s, "HADES_CERBERUS_PORTA_TUNEL_HOSTS"), "");
}

#[test]
fn recusa_valor_invalido_sem_gravar_nada() {
    let s = sandbox();
    let fonte = ConfigLocal::new(&s.env);
    let linhas = fonte.carregar().unwrap();
    let mut t = textos(&linhas);
    t[0] = "bloqueia".into();
    t[1] = "601".into();
    t[2] = "a b".into();
    let e = erros(&linhas, &t);
    assert_eq!(e[0].as_deref(), Some("um de off, avisa, barra"));
    assert_eq!(
        e[1].as_deref(),
        Some("Informe um número inteiro entre 1 e 600")
    );
    assert!(e[2].as_deref().unwrap().starts_with("padrões de nome"));
    assert_eq!(
        salvar(&fonte, &linhas, &t),
        Err("Modo da Porta: um de off, avisa, barra".into())
    );
    assert_eq!(valor(&s, "HADES_CERBERUS_TIMEOUT_S"), "30");
}

#[test]
fn ambiente_vence_e_fica_so_leitura() {
    let s = sandbox();
    let env = s.env.clone().with("HADES_CERBERUS_PORTA", "off");
    let fonte = ConfigLocal::new(&env);
    let linhas = fonte.carregar().unwrap();
    assert_eq!(linhas[0].valor, "off");
    assert!(linhas[0].somente_leitura());
    let mut t = textos(&linhas);
    t[0] = "lixo".into();
    assert_eq!(pendentes(&linhas, &t), 0);
    assert_eq!(erros(&linhas, &t)[0], None);
    assert_eq!(salvar(&fonte, &linhas, &t), Ok(0));
}

#[test]
fn sem_instalacao_nao_grava() {
    let s = sandbox();
    fs::remove_file(s.cwd.join(".claude/hades/instance")).unwrap();
    let env = Env::empty()
        .with("HOME", s.cwd.to_string_lossy())
        .with("HADES_SKILL_DIR", s.cwd.join("skill").to_string_lossy());
    let fonte = ConfigLocal::new(&env);
    assert!(!fonte.gravavel());
    let linhas = fonte.carregar().unwrap();
    let mut t = textos(&linhas);
    t[1] = "40".into();
    assert!(salvar(&fonte, &linhas, &t).is_err());
}

#[test]
fn chave_no_config_env_com_o_valor_do_padrao_vem_do_arquivo() {
    let s = sandbox();
    let dados = s.cwd.join("dados");
    fs::write(dados.join("config.env"), "HADES_CERBERUS_PORTA=avisa\n").unwrap();
    let linhas = ConfigLocal::new(&s.env).carregar().unwrap();
    assert_eq!(linhas[0].valor, "avisa");
    assert_eq!(linhas[0].origem, Origem::ConfigEnv);
    assert_eq!(linhas[1].origem, Origem::Padrao);

    let exportada = s.env.clone().with("HADES_CERBERUS_PORTA", "");
    let linhas = ConfigLocal::new(&exportada).carregar().unwrap();
    assert_eq!(
        (linhas[0].valor.as_str(), linhas[0].origem),
        ("", Origem::Env)
    );
    assert!(linhas[0].somente_leitura());
}
