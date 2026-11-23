//! Os casos de `loop/test/ts/cerberus-ts.test.ts`, com o executor de verdade e guardas em bash na caixa de areia.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::json;

use super::*;
use crate::executor::{Pedido, Saida};
use crate::testes::Caixa;

fn cfg(vigilia: &str) -> Cfg {
    [
        ("HADES_CERBERUS_PORTA", "barra"),
        ("HADES_CERBERUS_LIMIAR", "barra"),
        ("HADES_CERBERUS_VIGILIA", vigilia),
        ("HADES_CERBERUS_TIMEOUT_S", "30"),
        ("HADES_CERBERUS_HIST_KB", "1024"),
        ("HADES_CERBERUS_EXC_TTL_H", "24"),
        ("HADES_JOURNAL", "0"),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .collect()
}

fn ev(res: &str) -> String {
    json!({"resultado": res, "evidencia": [{"conferido": format!("conferi {res}"), "onde": "teste.sh"}], "detalhe": "d"})
        .to_string()
}

/// Guarda exec `<cab>/<id>` com o script `checks/<id>.sh` (bash) e os campos extras dados.
fn exec(caixa: &Caixa, cab: &str, id: &str, momentos: &[&str], corpo: &str, extra: Value) {
    let script = caixa.escreve(
        &format!("skill/loop/cerberus/{cab}/checks/{id}.sh"),
        &format!("#!/bin/bash\n{corpo}\n"),
    );
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let mut g = json!({"titulo": format!("T {id}"), "origem": "teste", "momentos": momentos, "tipo": "exec",
        "cmd": [format!("checks/{id}.sh")]});
    if let (Value::Object(g), Value::Object(extra)) = (&mut g, extra) {
        g.extend(extra);
    }
    caixa.guarda(cab, id, &g.to_string());
}

fn emitida(caixa: &Caixa) {
    caixa.guarda(
        "vigilia",
        "so-dono",
        r#"{"titulo": "Só o dono", "origem": "teste", "momentos": ["continuo"], "tipo": "emitida"}"#,
    );
}

fn entrada(cfg_: Cfg, ctx: &[(&str, &str)]) -> Entrada {
    Entrada {
        ctx: ctx
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        cfg: Some(cfg_),
        ..Entrada::default()
    }
}

fn do_id<'a>(a: &'a Avaliacao, id: &str) -> &'a Dict {
    a.vereditos
        .iter()
        .find(|v| v["guarda"] == id)
        .unwrap_or_else(|| panic!("sem veredito de {id}"))
}

fn conferido(v: &Dict) -> &str {
    v["evidencia"][0]["conferido"].as_str().unwrap()
}

fn guardas_do_ts(caixa: &Caixa) {
    let tsv: String = cfg("barra")
        .iter()
        .map(|(k, v)| format!("{k}\t{v}\n"))
        .collect();
    caixa.escreve("skill/kit/config.tsv", &tsv);
    for (id, res) in [("t-o", "ok"), ("t-a", "aviso"), ("t-b", "barra")] {
        exec(
            caixa,
            "porta",
            id,
            &["release", "done", "continuo", "push"],
            &format!("echo '{}'", ev(res)),
            json!({}),
        );
    }
    exec(
        caixa,
        "limiar",
        "t-sem-evidencia",
        &["release"],
        r#"echo '{"resultado": "ok"}'"#,
        json!({}),
    );
    exec(
        caixa,
        "limiar",
        "t-lixo",
        &["release"],
        "echo nao-e-json",
        json!({}),
    );
    exec(caixa, "limiar", "t-saiu-1", &["done"], "exit 1", json!({}));
    exec(caixa, "limiar", "t-nada", &["continuo"], "true", json!({}));
    exec(
        caixa,
        "limiar",
        "t-outro-id",
        &["release"],
        r#"echo '{"guarda": "outra", "resultado": "ok", "evidencia": [{"conferido": "a", "onde": "b"}]}'"#,
        json!({}),
    );
    exec(
        caixa,
        "vigilia",
        "t-ctx",
        &["release"],
        r#"echo "{\"resultado\": \"ok\", \"evidencia\": [{\"conferido\": \"$CERBERUS_CTX_PR $CERBERUS_MOMENTO $CERBERUS_CABECA $(sed 's/"/\\"/g')\", \"onde\": \"$(basename "$CERBERUS_DIR")\"}]}""#,
        json!({}),
    );
    exec(caixa, "vigilia", "t-sem-x", &["release"], "true", json!({}));
    let sem_x = caixa
        .raiz()
        .join("skill/loop/cerberus/vigilia/checks/t-sem-x.sh");
    fs::set_permissions(sem_x, fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn avaliar_release_como_o_ts() {
    let caixa = Caixa::nova();
    guardas_do_ts(&caixa);
    let c = Cerberus::abrir(caixa.opcoes().clone());
    let a = c
        .avaliar("release", entrada(cfg("barra"), &[("pr", "7")]))
        .unwrap();
    assert_eq!(a.vereditos.len(), 8);
    assert!(a.barrou());
    assert_eq!((a.barras, a.avisos), (5, 1));
    assert_eq!(do_id(&a, "t-o")["efeito"], "ok");
    assert_eq!(do_id(&a, "t-a")["efeito"], "aviso");
    let b = do_id(&a, "t-b");
    assert_eq!(b["efeito"], "barra");
    assert_eq!(
        b.keys().collect::<Vec<_>>(),
        [
            "ts",
            "avaliacao",
            "resultado",
            "evidencia",
            "detalhe",
            "cabeca",
            "guarda",
            "momento",
            "modo",
            "efeito",
            "origem",
            "ctx",
            "dir"
        ]
    );
    assert_eq!(b["ctx"], json!({"pr": "7"}));
    assert_eq!(b["dir"], caixa.raiz().to_string_lossy().as_ref());
    for (id, why) in [
        ("t-sem-evidencia", "veredito sem evidência"),
        ("t-lixo", "saída sem veredito JSON"),
        ("t-outro-id", "guarda='outra' não bate com 't-outro-id'"),
        ("t-sem-x", "não executou: Permission denied"),
    ] {
        let v = do_id(&a, id);
        assert_eq!(v["efeito"], "barra", "{id}");
        assert_eq!(v["invalido"], true, "{id}");
        assert_eq!(conferido(v), format!("veredito recusado: {why}"), "{id}");
    }
    assert_eq!(
        do_id(&a, "t-sem-x")["evidencia"][0]["onde"],
        "loop/cerberus/vigilia/checks/t-sem-x.sh"
    );
    let ctx = do_id(&a, "t-ctx");
    assert_eq!(ctx["efeito"], "ok");
    let nome = caixa.raiz().file_name().unwrap().to_string_lossy();
    assert_eq!(ctx["evidencia"][0]["onde"], nome.as_ref());
    assert_eq!(conferido(ctx), r#"7 release vigilia {"pr":"7"}"#);

    let hist = c.historico(None, None, None).unwrap();
    assert_eq!(hist.len(), 8);
    assert!(hist.iter().all(|r| r["avaliacao"] == a.avaliacao.as_str()));
}

#[test]
fn saiu_sem_veredito_e_saida_vazia() {
    let caixa = Caixa::nova();
    guardas_do_ts(&caixa);
    let c = Cerberus::abrir(caixa.opcoes().clone());
    let a = c.avaliar("done", entrada(cfg("barra"), &[])).unwrap();
    assert_eq!(
        conferido(do_id(&a, "t-saiu-1")),
        "veredito recusado: saiu 1 sem veredito"
    );
    let a = c.avaliar("continuo", entrada(cfg("barra"), &[])).unwrap();
    assert_eq!(
        conferido(do_id(&a, "t-nada")),
        "veredito recusado: saída sem veredito JSON"
    );
}

#[test]
fn guarda_lenta_passa_do_tempo() {
    let caixa = Caixa::nova();
    exec(
        &caixa,
        "vigilia",
        "t-lenta",
        &["release"],
        "sleep 6",
        json!({"timeoutS": 1}),
    );
    let c = Cerberus::abrir(caixa.opcoes().clone());
    let inicio = Instant::now();
    let a = c.avaliar("release", entrada(cfg("barra"), &[])).unwrap();
    assert!(inicio.elapsed().as_secs() < 5);
    assert_eq!(
        conferido(do_id(&a, "t-lenta")),
        "veredito recusado: passou de 1 s"
    );
}

#[test]
fn saida_de_70_kib_perde_o_veredito() {
    let caixa = Caixa::nova();
    exec(
        &caixa,
        "porta",
        "t-grande",
        &["push"],
        &format!(
            "head -c 71680 /dev/zero | tr '\\0' x; echo; echo '{}'",
            ev("ok")
        ),
        json!({}),
    );
    exec(
        &caixa,
        "porta",
        "t-grande-ok",
        &["push"],
        &format!(
            "echo; echo '{}'; head -c 100 /dev/zero | tr '\\0' ' '",
            ev("ok")
        ),
        json!({}),
    );
    let c = Cerberus::abrir(caixa.opcoes().clone());
    let a = c.avaliar("push", entrada(cfg("barra"), &[])).unwrap();
    assert_eq!(
        conferido(do_id(&a, "t-grande")),
        "veredito recusado: saída sem veredito JSON"
    );
    assert_eq!(do_id(&a, "t-grande-ok")["efeito"], "ok");
}

#[test]
fn cabeca_em_off_e_filtro_de_cabeca() {
    let caixa = Caixa::nova();
    guardas_do_ts(&caixa);
    let c = Cerberus::abrir(caixa.opcoes().clone());
    let a = c.avaliar("release", entrada(cfg("off"), &[])).unwrap();
    assert_eq!(a.ignoradas, ["vigilia"]);
    assert!(a.vereditos.iter().all(|v| v["cabeca"] != "vigilia"));
    let mut e = entrada(cfg("off"), &[]);
    e.cabeca = Some("porta".into());
    let a = c.avaliar("release", e).unwrap();
    assert!(a.ignoradas.is_empty());
    assert_eq!(a.vereditos.len(), 3);
    let a = c.avaliar("plugin", entrada(cfg("barra"), &[])).unwrap();
    assert!(a.vereditos.is_empty());
    assert_eq!(a.resultado, Resultado::Ok);
}

#[test]
fn modo_avisa_rebaixa_o_barra() {
    let caixa = Caixa::nova();
    guardas_do_ts(&caixa);
    let c = Cerberus::abrir(caixa.opcoes().clone());
    let mut cfg_ = cfg("avisa");
    cfg_.insert("HADES_CERBERUS_PORTA".into(), "avisa".into());
    let mut e = entrada(cfg_, &[]);
    e.cabeca = Some("porta".into());
    let a = c.avaliar("release", e).unwrap();
    assert_eq!(do_id(&a, "t-b")["efeito"], "aviso");
    assert_eq!(do_id(&a, "t-b")["modo"], "avisa");
    assert_eq!(a.resultado, Resultado::Aviso);
}

fn recusa(r: Result<impl std::fmt::Debug, Erro>) -> String {
    match r {
        Err(Erro::Recusa(m)) => m,
        outro => panic!("esperava recusa: {outro:?}"),
    }
}

#[test]
fn recusas_na_ordem_do_ts() {
    let caixa = Caixa::nova();
    guardas_do_ts(&caixa);
    let c = Cerberus::abrir(caixa.opcoes().clone());
    assert_eq!(
        recusa(c.avaliar("bogus", entrada(cfg("barra"), &[]))),
        format!("momento inválido: bogus ({})", Momento::lista())
    );
    let mut e = entrada(cfg("barra"), &[]);
    e.cabeca = Some("x".into());
    assert_eq!(recusa(c.avaliar("push", e)), "cabeça inválida: x");
    assert_eq!(
        recusa(c.avaliar("release", entrada(cfg("talvez"), &[]))),
        "HADES_CERBERUS_VIGILIA='talvez': modo inválido (off, avisa ou barra)"
    );
    let mut zero = cfg("barra");
    zero.insert("HADES_CERBERUS_TIMEOUT_S".into(), "0".into());
    assert_eq!(
        recusa(c.avaliar("release", entrada(zero, &[]))),
        "HADES_CERBERUS_TIMEOUT_S=0 fora de 1–600"
    );
    caixa.guarda(
        "porta",
        "t-quebrada",
        r#"{"titulo": "x", "momentos": ["push"], "tipo": "magia", "origem": "teste"}"#,
    );
    assert!(
        recusa(c.avaliar("push", entrada(cfg("barra"), &[])))
            .starts_with("registro do Cerberus inválido: porta/t-quebrada.json: tipo: ")
    );
}

#[test]
fn excecao_com_alvo_e_uso_unico() {
    let caixa = Caixa::nova();
    guardas_do_ts(&caixa);
    let c = Cerberus::abrir(caixa.opcoes().clone());
    let exc = c
        .abrir_excecao(PedidoExcecao {
            cabeca: "porta".into(),
            guarda: "t-b".into(),
            momento: "push".into(),
            motivo: "teste de paridade".into(),
            alvo: vec!["pr=9".into()],
            origem: "terminal".into(),
            ttl_s: Some(3600),
            ..PedidoExcecao::default()
        })
        .unwrap();
    let id = exc.id().unwrap().to_owned();
    let porta = |pr: &str| {
        let mut e = entrada(cfg("barra"), &[("pr", pr)]);
        e.cabeca = Some("porta".into());
        c.avaliar("push", e).unwrap()
    };
    assert_eq!(do_id(&porta("1"), "t-b")["efeito"], "barra");
    let a = porta("9");
    let b = do_id(&a, "t-b");
    assert_eq!(b["efeito"], "liberado");
    assert_eq!(a.resultado, Resultado::Aviso);
    assert_eq!(
        b["excecao"].as_object().unwrap().keys().collect::<Vec<_>>(),
        ["id", "motivo", "por", "aberta"]
    );
    assert_eq!(b["excecao"]["id"], id.as_str());
    assert_eq!(b["excecao"]["por"], "dono");
    assert_eq!(do_id(&porta("9"), "t-b")["efeito"], "barra");
    let excs = c.excecoes().unwrap();
    assert_eq!(excs[0].texto("usada"), Some(a.avaliacao.as_str()));
    assert!(excs[0].texto("usadaEm").is_some());
}

#[test]
fn excecao_sem_alvo_e_recusas() {
    let caixa = Caixa::nova();
    guardas_do_ts(&caixa);
    let c = Cerberus::abrir(caixa.opcoes().clone());
    let pedido = |guarda: &str, momento: &str, motivo: &str, alvo: &[&str]| PedidoExcecao {
        cabeca: "porta".into(),
        guarda: guarda.into(),
        momento: momento.into(),
        motivo: motivo.into(),
        alvo: alvo.iter().map(|s| (*s).to_owned()).collect(),
        origem: "terminal".into(),
        ttl_s: Some(3600),
        ..PedidoExcecao::default()
    };
    c.abrir_excecao(pedido("t-b", "done", "teste de paridade", &[]))
        .unwrap();
    let a = c
        .avaliar("done", entrada(cfg("barra"), &[("pr", "3")]))
        .unwrap();
    assert_eq!(do_id(&a, "t-b")["efeito"], "liberado");
    assert_eq!(
        recusa(c.abrir_excecao(pedido("nao-existe", "push", "teste de paridade", &[]))),
        "guarda porta/nao-existe não registrada"
    );
    assert_eq!(
        recusa(c.abrir_excecao(pedido("t-b", "x", "teste de paridade", &[]))),
        "momento inválido: x"
    );
    assert_eq!(
        recusa(c.abrir_excecao(pedido("t-b", "push", "curto", &[]))),
        "motivo obrigatório (10 a 500 caracteres)"
    );
    assert!(
        recusa(c.abrir_excecao(pedido("nao-existe", "push", "x", &["Ruim"])))
            .starts_with("--ctx 'Ruim': ")
    );
}

/// Mexe num pedido válido para provocar uma recusa.
type Ajuste = Box<dyn Fn(&mut PedidoRegistro)>;

fn registro_valido() -> PedidoRegistro {
    PedidoRegistro {
        cabeca: "vigilia".into(),
        guarda: "so-dono".into(),
        momento: "continuo".into(),
        resultado: "barra".into(),
        evidencia: json!([{"conferido": "regra conferida", "onde": "hook.py"}]),
        detalhe: Some("x".into()),
        ctx: [("a".to_owned(), "1".to_owned())].into_iter().collect(),
        cfg: Some(cfg("barra")),
        ..PedidoRegistro::default()
    }
}

#[test]
fn registrar_como_o_ts() {
    let caixa = Caixa::nova();
    guardas_do_ts(&caixa);
    emitida(&caixa);
    let c = Cerberus::abrir(caixa.opcoes().clone());
    let rec = c.registrar(registro_valido()).unwrap();
    assert_eq!(
        rec.keys().collect::<Vec<_>>(),
        [
            "ts",
            "avaliacao",
            "cabeca",
            "guarda",
            "momento",
            "resultado",
            "evidencia",
            "detalhe",
            "modo",
            "efeito",
            "origem",
            "ctx",
            "dir"
        ]
    );
    assert_eq!(rec["efeito"], "barra");
    assert_eq!(rec["origem"], "emitida");
    assert_eq!(c.historico(None, Some("barra"), None).unwrap(), [rec]);

    let mut p = registro_valido();
    p.efeito = Some("aviso".into());
    p.cwd = Some("rel/dir".into());
    let rec = c.registrar(p).unwrap();
    assert_eq!(
        (&rec["efeito"], &rec["dir"]),
        (&json!("aviso"), &json!("rel/dir"))
    );

    let casos: Vec<(Ajuste, &str)> = vec![
        (
            Box::new(|p| p.evidencia = json!([])),
            "veredito recusado: veredito sem evidência",
        ),
        (
            Box::new(|p| p.momento = "release".into()),
            "vigilia/so-dono não é registrada no momento release",
        ),
        (
            Box::new(|p| p.resultado = "talvez".into()),
            "veredito recusado: resultado tem de ser ok, aviso ou barra",
        ),
        (
            Box::new(|p| p.efeito = Some("x".into())),
            "efeito tem de ser ok, aviso ou barra",
        ),
        (
            Box::new(|p| {
                p.cabeca = "porta".into();
                p.guarda = "t-o".into();
            }),
            "porta/t-o não está registrada como guarda emitida",
        ),
    ];
    for (muda, msg) in casos {
        let mut p = registro_valido();
        muda(&mut p);
        assert_eq!(recusa(c.registrar(p)), msg);
    }
}

#[test]
fn historico_e_estado() {
    let caixa = Caixa::nova();
    guardas_do_ts(&caixa);
    emitida(&caixa);
    let c = Cerberus::abrir(caixa.opcoes().clone());
    c.avaliar("release", entrada(cfg("barra"), &[])).unwrap();
    c.registrar(registro_valido()).unwrap();
    c.registrar(registro_valido()).unwrap();
    let todos = c.historico(None, None, None).unwrap();
    assert_eq!(todos.len(), 10);
    assert_eq!(c.historico(None, None, Some(2)).unwrap(), todos[8..]);
    assert_eq!(c.historico(Some("porta"), Some(""), None).unwrap().len(), 3);

    let st = c.estado(true).unwrap();
    assert_eq!(
        st.keys().collect::<Vec<_>>(),
        [
            "cabecas",
            "guardas",
            "erros",
            "momentos",
            "historico",
            "total",
            "barrados",
            "excecoes",
            "store"
        ]
    );
    assert_eq!(st["total"], 10);
    assert_eq!(
        st["cabecas"][2],
        json!({"id": "vigilia", "nome": "Vigília", "modo": "barra", "guardas": 3})
    );
    let barrados = st["barrados"].as_array().unwrap();
    assert_eq!(barrados[0]["guarda"], "so-dono");
    assert_eq!(barrados[0]["vezes"], 2);
    assert_eq!(barrados[1]["guarda"], "t-b");
    assert_eq!(barrados[1]["vezes"], 1);
}

#[test]
fn estado_lista_excecoes_com_os_campos_do_painel() {
    let caixa = Caixa::nova();
    guardas_do_ts(&caixa);
    let c = Cerberus::abrir(caixa.opcoes().clone());
    let store = Store::de(caixa.opcoes()).unwrap();
    store
        .gravar_excecoes(&[Excecao(json!({"id": "a", "extra": 1})), Excecao(json!(3))])
        .unwrap();
    let mut env = caixa.opcoes().env.clone();
    for (k, v) in cfg("barra") {
        env = env.with(&k, v);
    }
    let mut op = caixa.opcoes().clone();
    op.env = env;
    let st = Cerberus::abrir(op).estado(true).unwrap();
    let e = &st["excecoes"];
    assert_eq!(e[0]["id"], "a");
    assert_eq!(e[0].as_object().unwrap().len(), 12);
    assert_eq!(e[1]["id"], Value::Null);
    fs::remove_file(caixa.raiz().join("skill/kit/config.tsv")).unwrap();
    assert!(c.estado(true).is_err());
}

struct Gravador(Mutex<Vec<Pedido>>);

impl Executor for Gravador {
    fn executar(&self, pedido: &Pedido) -> Saida {
        self.0.lock().unwrap().push(pedido.clone());
        Saida::Terminou {
            status: 0,
            stdout: ev("ok"),
        }
    }
}

#[test]
fn guarda_da_cli_roda_o_binario_do_check() {
    let caixa = Caixa::nova();
    caixa.guarda(
        "porta",
        "via-cli",
        r#"{"titulo": "T", "origem": "o", "momentos": ["push"], "tipo": "exec", "cmd": ["hades", "cerberus-check", "porta/x"]}"#,
    );
    caixa.guarda(
        "porta",
        "outra-cli",
        r#"{"titulo": "T", "origem": "o", "momentos": ["push"], "tipo": "exec", "cmd": ["hades", "lock", "status"]}"#,
    );
    let gravador = Arc::new(Gravador(Mutex::new(Vec::new())));
    let op = caixa.opcoes().clone().com_bin_check("/opt/cerberus".into());
    let c = Cerberus::abrir(op).com_executor(gravador.clone());
    let a = c
        .avaliar("push", entrada(cfg("barra"), &[("pr", "1")]))
        .unwrap();
    assert_eq!(a.resultado, Resultado::Ok);
    let pedidos = gravador.0.lock().unwrap();
    let argvs: Vec<_> = pedidos.iter().map(|p| p.argv.clone()).collect();
    assert!(argvs.contains(&vec![
        "/opt/cerberus".into(),
        "check".into(),
        "porta/x".into()
    ]));
    assert!(argvs.contains(&vec!["hades".into(), "lock".into(), "status".into()]));
    let p = &pedidos[0];
    assert_eq!(p.stdin, br#"{"pr":"1"}"#);
    assert!(p.env.contains(&("CERBERUS_CTX_PR".into(), "1".into())));
    assert!(
        p.env
            .contains(&("CERBERUS_CTX".into(), r#"{"pr":"1"}"#.into()))
    );
    assert_eq!(p.timeout.as_secs(), 30);
}

#[test]
fn ensaiar_roda_a_guarda_sem_gravar_o_historico() {
    let caixa = Caixa::nova();
    exec(
        &caixa,
        "porta",
        "t-a",
        &["push"],
        &format!("echo '{}'", ev("aviso")),
        json!({}),
    );
    emitida(&caixa);
    let gravador = Arc::new(Gravador(Mutex::new(Vec::new())));
    let c = Cerberus::abrir(caixa.opcoes().clone());
    let ctx: Ctx = [("texto".to_owned(), "abc".to_owned())]
        .into_iter()
        .collect();
    let v = c
        .ensaiar("porta", "t-a", "push", &ctx, Some(cfg("barra")))
        .unwrap();
    assert_eq!(v.resultado(), Some(Resultado::Aviso));
    assert_eq!(v.guarda(), Some("t-a"));
    let dir = Store::de(caixa.opcoes()).unwrap().dir().to_path_buf();
    assert!(!dir.join("vereditos.jsonl").exists());

    let c = c.com_executor(gravador.clone());
    c.ensaiar("porta", "t-a", "push", &ctx, Some(cfg("barra")))
        .unwrap();
    let pedidos = gravador.0.lock().unwrap();
    assert_eq!(pedidos[0].stdin, br#"{"texto":"abc"}"#);
    assert_eq!(pedidos[0].timeout.as_secs(), 30);
    drop(pedidos);

    let erro = |cab: &str, id: &str, m: &str| {
        c.ensaiar(cab, id, m, &ctx, Some(cfg("barra")))
            .unwrap_err()
            .to_string()
    };
    assert!(erro("porta", "nao-ha", "push").contains("não registrada"));
    assert!(erro("vigilia", "so-dono", "continuo").contains("não é exec"));
    assert!(erro("porta", "t-a", "release").contains("não roda em release"));
    assert!(erro("porta", "t-a", "xyz").contains("momento inválido"));
}
