use super::*;

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::process::Command;

use serde_json::json;

use crate::check::CheckVerdict;
use crate::limiar;

/// No CI os casos de bwrap são obrigatórios: com esta variável, bwrap ausente ou quebrado falha o teste em vez de pular.
const OBRIGATORIO: &str = "CERBERUS_BWRAP_OBRIGATORIO";

fn path_do_processo() -> String {
    std::env::var("PATH").unwrap_or_default()
}

/// `HAS_BWRAP` (`loop/test/ts/cerberus-ensaio.test.ts:10`), mais a sonda de que ele sobe.
fn tem_bwrap(caso: &str) -> bool {
    let ok = find_bwrap(Some(&path_do_processo())).is_some_and(|b| bwrap_works(&b).is_none());
    if !ok {
        assert!(
            std::env::var(OBRIGATORIO).map_or(true, |v| v.is_empty()),
            "{caso}: bwrap ausente ou sem user namespace, e {OBRIGATORIO} pede o ensaio"
        );
        eprintln!("aviso: {caso} pulado: bwrap ausente ou sem user namespace");
    }
    ok
}

fn tem(bin: &str) -> bool {
    hades_core::proc::which(bin, &path_do_processo()).is_some()
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// `Pack` e `mkPack` (`loop/test/ts/cerberus-ensaio.test.ts:16-37`): pacote git com o `env/` dado, mais um `$D` e um
/// HOME descartáveis.
struct Pack {
    t: tempfile::TempDir,
    pack: PathBuf,
    commit: String,
    data: PathBuf,
    home: PathBuf,
}

fn mk_pack(files: &[(&str, &str)], plugin_json: Value) -> Pack {
    let t = tempfile::tempdir().unwrap();
    let (pack, data, home) = (
        t.path().join("pack"),
        t.path().join("D"),
        t.path().join("home"),
    );
    for d in [&pack, &data, &home, &pack.join("env")] {
        fs::create_dir_all(d).unwrap();
    }
    git(&pack, &["init", "-q", "-b", "main"]);
    fs::write(
        pack.join("profile.json"),
        json!({"name": "ens", "env": {"pack": "env"}}).to_string(),
    )
    .unwrap();
    fs::write(pack.join("env/plugin.json"), plugin_json.to_string()).unwrap();
    for (rel, body) in files {
        let p = pack.join("env").join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, format!("#!/usr/bin/env bash\n{body}\n")).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    git(&pack, &["add", "-A"]);
    git(&pack, &["commit", "-qm", "v1"]);
    let commit = git(&pack, &["rev-parse", "HEAD"]);
    Pack {
        t,
        pack,
        commit,
        data,
        home,
    }
}

fn simples() -> Value {
    json!({"name": "ens"})
}

/// `ensaio` (`loop/test/ts/cerberus-ensaio.test.ts:39`): a guarda pelo Limiar, como a CLI a chama.
fn rodar(p: &Pack, extra: &[(&str, &str)]) -> CheckVerdict {
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let mut env: BTreeMap<String, String> = BTreeMap::from([
        ("PATH".into(), path_do_processo()),
        ("HOME".into(), s(&p.home)),
        ("HADES_DATA".into(), s(&p.data)),
        ("CERBERUS_MOMENTO".into(), "plugin".into()),
    ]);
    for (k, v) in extra {
        env.insert((*k).into(), (*v).into());
    }
    let mut ce = CheckEnv::do_ambiente(env, &p.pack);
    for (k, v) in [
        ("plugin", "ens".to_owned()),
        ("commit", p.commit.clone()),
        ("op", "enable".into()),
        ("pack_dir", s(&p.pack)),
    ] {
        ce.ctx.insert(k.into(), v);
    }
    limiar::rodar("ensaio", &ce).unwrap()
}

fn texto(v: &CheckVerdict) -> String {
    v.evidencia
        .iter()
        .map(|e| format!("{} @ {}", e.conferido, e.onde))
        .collect::<Vec<_>>()
        .join("\n")
}

// ------------------------------------------------------------ casos com bwrap

#[test]
fn post_install_que_so_escreve_no_d_passa_e_o_d_real_fica_intacto() {
    if !tem_bwrap("post-install no $D") {
        return;
    }
    let p = mk_pack(
        &[("post-install.sh", r#"touch "$HADES_DATA/ran""#)],
        simples(),
    );
    let v = rodar(&p, &[]);
    assert_eq!(v.resultado, Resultado::Ok, "{}", texto(&v));
    assert!(texto(&v).contains("post-install.sh: ensaiado"));
    assert!(!p.data.join("ran").exists());
}

#[test]
fn escrita_no_home_fora_do_d_barra_e_nomeia_o_caminho() {
    if !tem_bwrap("escrita no HOME") {
        return;
    }
    let p = mk_pack(
        &[("post-install.sh", r#"echo x > "$HOME/fora.txt""#)],
        simples(),
    );
    let v = rodar(&p, &[]);
    assert_eq!(v.resultado, Resultado::Barra);
    assert!(
        texto(&v).contains(&*p.home.join("fora.txt").to_string_lossy()),
        "{}",
        texto(&v)
    );
    assert!(!p.home.join("fora.txt").exists());
}

#[test]
fn escrita_em_pasta_irma_do_d_barra_com_o_caminho() {
    if !tem_bwrap("pasta irmã do $D") {
        return;
    }
    let p = mk_pack(
        &[("post-install.sh", r#"echo x > "$HADES_DATA/../solto.txt""#)],
        simples(),
    );
    let v = rodar(&p, &[]);
    assert_eq!(v.resultado, Resultado::Barra);
    assert!(texto(&v).contains("solto.txt"), "{}", texto(&v));
    assert!(!p.t.path().join("solto.txt").exists());
}

#[test]
fn rede_sem_declaracao_barra() {
    if !tem_bwrap("rede sem declaração") {
        return;
    }
    let p = mk_pack(
        &[("post-install.sh", "exec 3<>/dev/tcp/127.0.0.1/9")],
        simples(),
    );
    let v = rodar(&p, &[]);
    assert_eq!(v.resultado, Resultado::Barra);
    assert!(texto(&v).contains("rede"), "{}", texto(&v));
}

#[test]
fn rede_declarada_passa_e_registra_o_motivo() {
    if !tem_bwrap("rede declarada") {
        return;
    }
    let srv = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = srv.local_addr().unwrap().port();
    let p = mk_pack(
        &[(
            "post-install.sh",
            &format!("exec 3<>/dev/tcp/127.0.0.1/{port}"),
        )],
        json!({"name": "ens", "ensaio": {"rede": [{"item": "post-install.sh", "motivo": "baixa o asset do simulador"}]}}),
    );
    let v = rodar(&p, &[]);
    assert_eq!(v.resultado, Resultado::Ok, "{}", texto(&v));
    assert!(texto(&v).contains("rede liberada por declaração"));
    assert!(texto(&v).contains("baixa o asset do simulador"));
    assert!(
        v.detalhe
            .unwrap()
            .contains("1 com rede liberada por declaração")
    );
}

#[test]
fn script_que_falha_no_ensaio_barra() {
    if !tem_bwrap("script que falha") {
        return;
    }
    let p = mk_pack(&[("post-install.sh", "exit 3")], simples());
    let v = rodar(&p, &[]);
    assert_eq!(v.resultado, Resultado::Barra);
    assert!(texto(&v).contains("post-install.sh"));
    assert!(texto(&v).contains("saiu com 3"), "{}", texto(&v));
    assert_eq!(
        v.detalhe.as_deref(),
        Some("1 script(s) barrado(s) no ensaio")
    );
}

#[test]
fn script_que_chama_docker_nao_e_ensaiado_e_nao_barra() {
    if !tem_bwrap("docker") {
        return;
    }
    let p = mk_pack(&[("post-install.sh", "docker compose up -d")], simples());
    let v = rodar(&p, &[]);
    assert_eq!(v.resultado, Resultado::Ok);
    assert!(texto(&v).contains("não ensaiado (docker)"), "{}", texto(&v));
    assert_eq!(
        v.detalhe.as_deref(),
        Some("0 script(s) ensaiado(s), 1 não ensaiado(s)")
    );
}

#[test]
fn doctor_e_checks_tambem_sao_ensaiados() {
    if !tem_bwrap("doctor e checks") {
        return;
    }
    let p = mk_pack(
        &[
            ("doctor.sh", r#"ok G x "tudo""#),
            ("checks/a.sh", "echo feito"),
        ],
        simples(),
    );
    let v = rodar(&p, &[]);
    assert_eq!(v.resultado, Resultado::Ok, "{}", texto(&v));
    assert!(texto(&v).contains("doctor.sh: ensaiado"));
    assert!(texto(&v).contains("checks/a.sh: ensaiado"));
    assert_eq!(
        v.evidencia[0].conferido.split(':').next(),
        Some("checks/a.sh")
    );
}

// ------------------------------------------------------------ escrita declarada em ensaio.escrita[]

/// `mkWt` (`loop/test/ts/cerberus-ensaio.test.ts:120`).
fn mk_wt(p: &Pack) -> PathBuf {
    let wt = p.t.path().join("wt");
    fs::create_dir_all(wt.join("tools")).unwrap();
    fs::write(wt.join("tools/velho.txt"), "v").unwrap();
    wt
}

fn decl(caminho: &str) -> Value {
    json!({"name": "ens", "ensaio": {"escrita": [{"item": "post-install.sh", "caminho": caminho, "motivo": "extrai o simulador"}]}})
}

#[test]
fn escrita_no_worktree_sem_declaracao_barra_com_o_caminho() {
    if !tem_bwrap("worktree sem declaração") {
        return;
    }
    let p = mk_pack(
        &[("post-install.sh", r#"echo x > "$HADES_WT/tools/novo.txt""#)],
        simples(),
    );
    let wt = mk_wt(&p);
    let v = rodar(&p, &[("HADES_WT", &wt.to_string_lossy())]);
    assert_eq!(v.resultado, Resultado::Barra);
    assert!(
        texto(&v).contains(&*wt.join("tools/novo.txt").to_string_lossy()),
        "{}",
        texto(&v)
    );
}

#[test]
fn escrita_no_caminho_declarado_passa_e_o_worktree_real_fica_intacto() {
    if !tem_bwrap("escrita declarada") {
        return;
    }
    let p = mk_pack(
        &[(
            "post-install.sh",
            r#"echo x > "$HADES_WT/tools/novo.txt"; test -f "$HADES_WT/tools/velho.txt""#,
        )],
        decl("tools"),
    );
    let wt = mk_wt(&p);
    let v = rodar(&p, &[("HADES_WT", &wt.to_string_lossy())]);
    assert_eq!(v.resultado, Resultado::Ok, "{}", texto(&v));
    assert!(texto(&v).contains(
        "escrita liberada por declaração (cópia descartável): tools (extrai o simulador)"
    ));
    assert!(!wt.join("tools/novo.txt").exists());
}

#[test]
fn o_caminho_declarado_nao_abre_o_resto_do_worktree() {
    if !tem_bwrap("resto do worktree") {
        return;
    }
    let p = mk_pack(
        &[("post-install.sh", r#"echo x > "$HADES_WT/outro.txt""#)],
        decl("tools"),
    );
    let wt = mk_wt(&p);
    let v = rodar(&p, &[("HADES_WT", &wt.to_string_lossy())]);
    assert_eq!(v.resultado, Resultado::Barra);
    assert!(texto(&v).contains("outro.txt"));
}

#[test]
fn caminho_declarado_que_sai_do_worktree_e_ignorado_e_a_escrita_barra() {
    if !tem_bwrap("caminho fora do worktree") {
        return;
    }
    let p = mk_pack(
        &[("post-install.sh", r#"echo x > "$HADES_WT/tools/novo.txt""#)],
        decl("../tools"),
    );
    let wt = mk_wt(&p);
    let v = rodar(&p, &[("HADES_WT", &wt.to_string_lossy())]);
    assert_eq!(v.resultado, Resultado::Barra);
}

#[test]
fn a_declaracao_vale_so_para_o_item_nomeado() {
    if !tem_bwrap("item nomeado") {
        return;
    }
    let p = mk_pack(
        &[("checks/a.sh", r#"echo x > "$HADES_WT/tools/novo.txt""#)],
        decl("tools"),
    );
    let wt = mk_wt(&p);
    let v = rodar(&p, &[("HADES_WT", &wt.to_string_lossy())]);
    assert_eq!(v.resultado, Resultado::Barra);
}

// ------------------------------------------------------------ sockets do host

#[test]
fn curl_no_docker_sock_do_host_nao_alcanca_o_daemon() {
    if !Path::new("/var/run/docker.sock").exists() || !tem("curl") || !tem_bwrap("docker.sock") {
        return;
    }
    let p = mk_pack(
        &[(
            "post-install.sh",
            "curl -fsS --max-time 5 --unix-socket /var/run/docker.sock http://x/version >/dev/null",
        )],
        simples(),
    );
    let v = rodar(&p, &[]);
    assert_eq!(v.resultado, Resultado::Barra);
    assert!(texto(&v).contains("post-install.sh: barrado"));
}

#[test]
fn socket_unix_do_host_fora_do_d_nao_alcanca_o_servidor() {
    if !tem("curl") || !tem_bwrap("socket do host") {
        return;
    }
    let pasta = tempfile::tempdir().unwrap();
    let sock = pasta.path().join("host.sock");
    let srv = UnixListener::bind(&sock).unwrap();
    let p = mk_pack(
        &[(
            "post-install.sh",
            &format!(
                "curl -fsS --max-time 5 --unix-socket {} http://x/ >/dev/null",
                sock.display()
            ),
        )],
        simples(),
    );
    let v = rodar(&p, &[]);
    assert_eq!(v.resultado, Resultado::Barra);
    srv.set_nonblocking(true).unwrap();
    assert_eq!(srv.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
}

// ------------------------------------------------------------ sem bwrap e sem código

#[test]
fn sem_bwrap_no_path_vira_aviso_com_como_instalar() {
    let p = mk_pack(&[("post-install.sh", "true")], simples());
    let bin = p.t.path().join("bin-sem-bwrap");
    fs::create_dir_all(&bin).unwrap();
    let real_git = hades_core::proc::which("git", &path_do_processo()).unwrap();
    symlink(real_git, bin.join("git")).unwrap();
    let v = rodar(&p, &[("PATH", &bin.to_string_lossy())]);
    assert_eq!(v.resultado, Resultado::Aviso, "{}", texto(&v));
    assert!(texto(&v).contains("bubblewrap"));
    assert!(texto(&v).contains("não ensaiado"));
    assert_eq!(
        v.detalhe.as_deref(),
        Some("ensaio não feito: sem bubblewrap utilizável")
    );
}

#[test]
fn plugin_sem_codigo_ensaiavel_passa_dizendo_o_que_nao_havia() {
    let p = mk_pack(&[], simples());
    let v = rodar(&p, &[]);
    assert_eq!(v.resultado, Resultado::Ok);
    assert!(texto(&v).contains("nada a ensaiar"));
    assert_eq!(v.detalhe.as_deref(), Some("sem código ensaiável"));
}

#[test]
fn plugin_json_ilegivel_barra_dizendo_o_que_faltou() {
    let p = mk_pack(&[("post-install.sh", "true")], json!(null));
    fs::write(p.pack.join("env/plugin.json"), "{").unwrap();
    git(&p.pack, &["commit", "-qam", "v2"]);
    let p = Pack {
        commit: git(&p.pack, &["rev-parse", "HEAD"]),
        ..p
    };
    let v = rodar(&p, &[]);
    assert_eq!(v.resultado, Resultado::Barra);
    assert!(texto(&v).contains("plugin.json do commit"), "{}", texto(&v));
}

// ------------------------------------------------------------ partes puras

#[test]
fn rede_declarada_exige_item_e_motivo() {
    let m = rede_declarada(&json!({"ensaio": {"rede": [
        {"item": "a.sh", "motivo": "  baixa  "},
        {"item": "b.sh", "motivo": "   "},
        {"item": "", "motivo": "x"},
        {"motivo": "x"},
        "texto",
        null,
    ]}}));
    assert_eq!(
        m.into_iter().collect::<Vec<_>>(),
        vec![("a.sh".into(), "baixa".into())]
    );
    assert!(rede_declarada(&json!(null)).is_empty());
    assert!(rede_declarada(&json!({"ensaio": {"rede": {}}})).is_empty());
}

#[test]
fn escrita_declarada_normaliza_e_recusa_o_que_sai_do_worktree() {
    let m = escrita_declarada(&json!({"ensaio": {"escrita": [
        {"item": "p.sh", "caminho": "tools/", "motivo": "m"},
        {"item": "p.sh", "caminho": "./a//b/../c", "motivo": " n "},
        {"item": "p.sh", "caminho": "../fora", "motivo": "m"},
        {"item": "p.sh", "caminho": "a/../..", "motivo": "m"},
        {"item": "p.sh", "caminho": "/abs", "motivo": "m"},
        {"item": "p.sh", "caminho": ".", "motivo": "m"},
        {"item": "p.sh", "caminho": "", "motivo": "m"},
        {"item": "p.sh", "caminho": "x", "motivo": ""},
    ]}}));
    let ditas: Vec<(&str, &str)> = m["p.sh"]
        .iter()
        .map(|e| (e.caminho.as_str(), e.motivo.as_str()))
        .collect();
    assert_eq!(ditas, vec![("tools", "m"), ("a/c", "n")]);
}

#[test]
fn normalize_como_o_node() {
    assert_eq!(normalize(""), ".");
    assert_eq!(normalize("a/./b/"), "a/b/");
    assert_eq!(normalize("../a/../../b"), "../../b");
    assert_eq!(normalize("//a/../.."), "/");
    assert_eq!(normalize("a/.."), ".");
}

#[test]
fn docker_chamado_e_docker_sondado() {
    assert!(calls_docker("docker compose up -d"));
    assert!(calls_docker("x; sudo docker run y"));
    assert!(calls_docker("  docker-compose up"));
    assert!(calls_docker("v=$(docker ps)"));
    assert!(calls_docker("echo `docker ps`"));
    assert!(calls_docker("docker"));
    assert!(!calls_docker("# docker run"));
    assert!(!calls_docker("command -v docker && docker ps"));
    assert!(!calls_docker("have docker || exit 0"));
    assert!(!calls_docker("echo dockerfile"));
    assert!(!calls_docker("mydocker run"));
}

#[test]
fn caminhos_gravados_vem_da_saida_de_erro() {
    let out = "touch: cannot touch '/h/fora.txt': Read-only file system\n\
               bash: /h/b.txt: Read-only file system\n\
               bash: /h/b.txt: Read-only file system\n\
               mkdir: Read-only file system\n\
               outra linha '/nada'";
    assert_eq!(written_paths(out), vec!["/h/fora.txt", "/h/b.txt"]);
}

#[test]
fn tail_junta_as_duas_ultimas_linhas() {
    assert_eq!(tail("  a\nb\nc  \n"), "b ⏎ c");
    assert_eq!(tail(""), "");
    let longo = "é".repeat(200);
    assert_eq!(tail(&longo).chars().count(), TAIL);
}

#[test]
fn pastas_que_somem_sem_filhas_de_run_e_tmp() {
    let v = volatile_dirs(Some("/run/user/0"));
    assert!(
        v.iter()
            .all(|d| !d.starts_with("/run/") && !d.starts_with("/tmp/"))
    );
    assert!(v.contains(&"/tmp".to_owned()));
    let t = tempfile::tempdir_in("/var/tmp").unwrap();
    let real = fs::canonicalize(t.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(volatile_dirs(Some(&real)).contains(&real));
    assert!(!volatile_dirs(Some("relativa")).contains(&"relativa".to_owned()));
}

#[test]
fn argumentos_do_bwrap_na_ordem_do_ts() {
    let sb = Sandbox {
        bwrap: "/usr/bin/bwrap".into(),
        copy: "/w/D".into(),
        target: "/d".into(),
        env: Vec::new(),
        home: "/h".into(),
        worktree: "/h/wt".into(),
        skill: String::new(),
        volatile: vec!["/run".into(), "/tmp".into()],
        sockets: vec!["/etc/s.sock".into()],
    };
    let writes = [WriteBind {
        copy: "/w/W0-0".into(),
        target: "/h/wt/tools".into(),
    }];
    let args = bwrap_args(
        &sb,
        false,
        &writes,
        "/d/.ensaio-env",
        &["bash", "/d/.ensaio-env/p.sh"],
    );
    let esperado = "--die-with-parent --unshare-pid --unshare-net --ro-bind / / --dev /dev --proc /proc \
                    --tmpfs /run --tmpfs /tmp --tmpfs /h --bind /w/D /d \
                    --bind /w/W0-0 /h/wt/tools --ro-bind /dev/null /etc/s.sock \
                    --remount-ro /run --remount-ro /tmp --remount-ro /h \
                    --chdir /d/.ensaio-env -- bash /d/.ensaio-env/p.sh";
    // o worktree sob o HOME volta somente leitura só se existir: aqui não existe
    assert_eq!(args.join(" "), esperado);
}

#[test]
fn find_bwrap_so_aceita_arquivo() {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir_all(t.path().join("a/bwrap")).unwrap();
    fs::create_dir_all(t.path().join("b")).unwrap();
    fs::write(t.path().join("b/bwrap"), "").unwrap();
    let path = format!(
        "::{}:{}",
        t.path().join("a").display(),
        t.path().join("b").display()
    );
    assert_eq!(find_bwrap(Some(&path)), Some(t.path().join("b/bwrap")));
    assert_eq!(find_bwrap(None), None);
}
