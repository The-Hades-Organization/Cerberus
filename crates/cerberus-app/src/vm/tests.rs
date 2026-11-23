use cerberus_core::modelo::{Cabeca, Dict, Modo, now_iso};
use cerberus_core::registro::Guarda;
use hades_ui::Tone;
use serde_json::{Value, json};

use super::*;

fn dict(v: Value) -> Dict {
    v.as_object().cloned().unwrap()
}

fn registro(ts: &str, cabeca: &str, guarda: &str, efeito: &str) -> Dict {
    dict(json!({
        "ts": ts,
        "avaliacao": "a1",
        "cabeca": cabeca,
        "guarda": guarda,
        "momento": "push",
        "resultado": efeito,
        "evidencia": [{"conferido": "API_KEY=abc123", "onde": "src/a.rs:4", "trecho": "API_KEY=abc123"}],
        "detalhe": "achou TOKEN=xyz",
        "modo": "barra",
        "efeito": efeito,
    }))
}

#[test]
fn tarefa_descarta_geracao_velha() {
    let mut t = Tarefa::default();
    let g = t.iniciar().unwrap();
    assert!(t.carregando());
    assert_eq!(t.iniciar(), None);
    t.invalidar();
    assert!(!t.carregando());
    let g2 = t.iniciar().unwrap();
    assert!(!t.concluir(g));
    assert!(t.carregando());
    assert!(t.concluir(g2));
    assert!(!t.carregando());
}

#[test]
fn telas_na_ordem_do_trilho() {
    assert_eq!(tela_da_tecla("1"), Some(Tela::Vereditos));
    assert_eq!(tela_da_tecla("6"), Some(Tela::Configuracoes));
    assert_eq!(tela_da_tecla("0"), None);
    assert_eq!(tela_da_tecla("7"), None);
    assert_eq!(tela_da_tecla("a"), None);
    for t in Tela::TODAS {
        assert_eq!(Tela::de_rotulo(t.rotulo()), Some(t));
    }
    assert_eq!(Tela::de_nome("modos"), Some(Tela::Modos));
    assert_eq!(Tela::de_nome("x"), None);
    assert!(Tela::Excecoes.pendente() && Tela::Testar.pendente() && Tela::Configuracoes.pendente());
    assert!(!Tela::Vereditos.pendente());
}

#[test]
fn filtro_olha_o_efeito() {
    let barra = registro("t", "porta", "g", "barra");
    let aviso = registro("t", "porta", "g", "aviso");
    assert!(Filtro::Barrados.aceita(&barra) && !Filtro::Barrados.aceita(&aviso));
    assert!(Filtro::Avisos.aceita(&aviso) && !Filtro::Avisos.aceita(&barra));
    assert!(Filtro::Todos.aceita(&barra));
    assert_eq!(Filtro::de_indice(2), Filtro::Avisos);
    assert_eq!(Filtro::de_indice(9), Filtro::Todos);
    assert_eq!(Filtro::Barrados.indice(), 1);
}

#[test]
fn tons_do_veredito() {
    assert_eq!(veredito("barra"), ("barrado", Tone::Bad));
    assert_eq!(veredito("aviso"), ("aviso", Tone::Warn));
    assert_eq!(veredito("ok"), ("passa", Tone::Ok));
    assert_eq!(veredito("liberado"), ("liberado", Tone::Info));
    assert_eq!(veredito("?"), ("—", Tone::Neutral));
}

#[test]
fn hora_e_data() {
    assert_eq!(hora("2026-11-21T13:02:44.000Z"), "13:02");
    assert_eq!(data_hora("2026-11-21T13:02:44.000Z"), "2026-11-21 13:02:44");
    assert_eq!(hora("ontem"), "ontem");
}

#[test]
fn mascara_valor_e_token_mas_deixa_o_caminho() {
    assert_eq!(mascarar("API_KEY=abc123"), "API_KEY=●●●●●●●●");
    assert_eq!(
        mascarar("token ghp1234567890abcdefXYZ em uso"),
        "token ●●●●●●●● em uso"
    );
    assert_eq!(mascarar("src/lib/a/b.rs:4"), "src/lib/a/b.rs:4");
    assert_eq!(mascarar("x= =y"), "x= =y");
}

#[test]
fn evidencia_nunca_traz_o_trecho() {
    let r = registro("t", "porta", "g", "barra");
    assert_eq!(
        evidencia(&r),
        vec![ItemEvidencia {
            onde: "src/a.rs:4".to_owned(),
            conferido: "API_KEY=●●●●●●●●".to_owned(),
        }]
    );
    let linha = linha_veredito(&r);
    assert_eq!(linha.evidencia, "src/a.rs:4");
    assert_eq!(linha.cabeca, "Porta");
    assert_eq!(linha.tom, Tone::Bad);
    assert_eq!(linha.celulas().len(), COLUNAS_VEREDITOS.len());
}

#[test]
fn inspecao_mascara_detalhe_e_mostra_excecao() {
    let mut r = registro("2026-11-21T13:02:44.000Z", "limiar", "segredos", "liberado");
    r.insert("excecao".to_owned(), json!({"id": "x-1"}));
    let i = inspecao(&r);
    assert_eq!(i.titulo, "segredos");
    assert_eq!(i.tom, Tone::Info);
    assert_eq!(i.detalhe.as_deref(), Some("achou TOKEN=●●●●●●●●"));
    assert!(i.meta.contains(&("Exceção", "x-1".to_owned())));
    assert!(i.meta.contains(&("Cabeça", "Limiar".to_owned())));
}

#[test]
fn visiveis_do_mais_novo_e_selecao() {
    let hist = vec![
        registro("1", "porta", "a", "barra"),
        registro("2", "porta", "b", "ok"),
        registro("3", "porta", "c", "barra"),
    ];
    assert_eq!(visiveis(&hist, Filtro::Todos), vec![2, 1, 0]);
    let barrados = visiveis(&hist, Filtro::Barrados);
    assert_eq!(barrados, vec![2, 0]);
    let alvo = chave(&hist[0]);
    assert_eq!(posicao_da_selecao(&hist, &barrados, Some(&alvo)), Some(1));
    let fora = chave(&hist[1]);
    assert_eq!(posicao_da_selecao(&hist, &barrados, Some(&fora)), None);
    assert_eq!(posicao_da_selecao(&hist, &barrados, None), None);
}

#[test]
fn kpis_de_24h_na_ordem_das_cabecas() {
    let agora = 1_800_000_000.0;
    let recente = now_iso(agora - 3600.0);
    let velho = now_iso(agora - 48.0 * 3600.0);
    let hist = vec![
        registro(&recente, "vigilia", "v", "barra"),
        registro(&recente, "porta", "p", "barra"),
        registro(&recente, "porta", "p", "ok"),
        registro(&velho, "porta", "p", "barra"),
    ];
    let k = kpis(&hist, agora);
    assert_eq!(
        k.iter().map(|k| k.cabeca).collect::<Vec<_>>(),
        CABECAS.to_vec()
    );
    assert_eq!((k[0].vereditos, k[0].barrados), (2, 1));
    assert_eq!((k[1].vereditos, k[1].barrados), (0, 0));
    assert_eq!((k[2].vereditos, k[2].barrados), (1, 1));
}

fn guarda(cab: Cabeca, id: &str, json: Value) -> Guarda {
    let mut g = dict(json);
    g.insert("cabeca".to_owned(), json!(cab.as_str()));
    g.insert("id".to_owned(), json!(id));
    Guarda::try_from(g).unwrap()
}

#[test]
fn guardas_por_cabeca() {
    let guardas = vec![
        guarda(
            Cabeca::Porta,
            "p1",
            json!({"tipo": "exec", "momentos": ["push", "done"], "cmd": ["checks/p1.sh", "-q"]}),
        ),
        guarda(
            Cabeca::Limiar,
            "l1",
            json!({"tipo": "regra", "momentos": ["push"]}),
        ),
    ];
    let porta = guardas_da_cabeca(&guardas, Cabeca::Porta);
    assert_eq!(
        porta,
        vec![LinhaGuarda {
            id: "p1".to_owned(),
            tipo: "exec".to_owned(),
            momentos: "push, done".to_owned(),
            cmd: "checks/p1.sh -q".to_owned(),
        }]
    );
    assert_eq!(guardas_da_cabeca(&guardas, Cabeca::Limiar)[0].cmd, "—");
    assert!(guardas_da_cabeca(&guardas, Cabeca::Vigilia).is_empty());
}

#[test]
fn erros_agrupados_na_ordem_das_cabecas() {
    let erros = vec![
        "vigilia/x.json: tipo".to_owned(),
        "solto".to_owned(),
        "porta/a.json: momentos".to_owned(),
        "porta/b.json: cmd".to_owned(),
    ];
    let g = erros_por_cabeca(&erros);
    assert_eq!(
        g.iter()
            .map(|(n, e)| (n.as_str(), e.len()))
            .collect::<Vec<_>>(),
        [("Porta", 2), ("Vigília", 1), ("Registro", 1)]
    );
}

#[test]
fn origem_env_vence_config_env_que_vence_padrao() {
    let chave = chave_do_modo(Cabeca::Limiar);
    assert_eq!(chave, "HADES_CERBERUS_LIMIAR");
    let arquivo = "OUTRA=1\nHADES_CERBERUS_LIMIAR=avisa\n";
    assert_eq!(origem_do_modo(&chave, true, Some(arquivo)), Origem::Env);
    assert_eq!(
        origem_do_modo(&chave, false, Some(arquivo)),
        Origem::ConfigEnv
    );
    assert_eq!(
        origem_do_modo(&chave, false, Some("HADES_CERBERUS_LIMIAR_X=1")),
        Origem::Padrao
    );
    assert_eq!(origem_do_modo(&chave, false, None), Origem::Padrao);
}

#[test]
fn modos_e_rodapes() {
    assert_eq!(rotulo_do_modo(Modo::Barra), ("barra", Tone::Bad));
    assert_eq!(rotulo_do_modo(Modo::Avisa), ("avisa", Tone::Warn));
    assert_eq!(rotulo_do_modo(Modo::Off), ("desligado", Tone::Neutral));
    assert_eq!(rodape_dos_vereditos(1), "1 veredito");
    assert_eq!(rodape_dos_vereditos(3), "3 vereditos");
    assert_eq!(rodape_das_guardas(2, 0), "2 guardas");
    assert_eq!(rodape_das_guardas(1, 1), "1 guarda · 1 erro");
}
