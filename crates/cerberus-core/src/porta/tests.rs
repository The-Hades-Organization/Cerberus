use super::*;

#[test]
fn segredos_e_falsos_positivos() {
    let token = format!("gh{}_{}", "p", "a1B2".repeat(9));
    assert_eq!(
        secret_in(&format!("olha {token}")).map(|h| h.0),
        Some("token do GitHub")
    );
    assert!(secret_in(&format!("token = \"{}\"", "x".repeat(20))).is_none());
    assert!(secret_in("url=https://u:<senha>@h").is_none());
    assert_eq!(
        secret_in(&format!("api_key: \"{}\"", "Zq9".repeat(8))).map(|h| h.0),
        Some("atribuição de segredo")
    );
    assert_eq!(mask("abcdefgh"), "abcd…(8 caracteres)");
}

#[test]
fn hunk_e_host() {
    assert_eq!(linha_do_hunk("@@ -1,0 +5,2 @@"), 4);
    assert_eq!(linha_do_hunk("@@ sem número"), 0);
    assert_eq!(host_of("https://A.devtunnels.ms/x"), "a.devtunnels.ms");
    assert_eq!(host_of("a.devtunnels.ms"), "a.devtunnels.ms");
    assert_eq!(host_of(""), "");
}

/// Equipamento dos casos de `test/ts/cerberus-porta-legado.test.ts`, como em `test/ts/lib/checks-rig.ts:13`: raiz
/// temporária, stubs de `gh` e `devtunnel` no PATH do `CheckEnv` e repositórios git de fixture.
mod rig {
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use indexmap::IndexMap;

    use crate::check::CheckEnv;

    pub struct Rig {
        pub t: tempfile::TempDir,
        n: Cell<usize>,
    }

    impl Rig {
        pub fn new() -> Self {
            let t = tempfile::tempdir().unwrap();
            for d in ["bin", "packs", "home", "work", "skill"] {
                fs::create_dir_all(t.path().join(d)).unwrap();
            }
            fs::create_dir_all(t.path().join("skill/kit")).unwrap();
            fs::write(
                t.path().join("skill/kit/config.tsv"),
                "HADES_CERBERUS_PORTA_TUNEL_HOSTS\t\tglobs\n",
            )
            .unwrap();
            let rig = Self { t, n: Cell::new(0) };
            rig.stub(
                "gh",
                r#"case "$2" in
  repos/*) [[ "$STUB_PROTEGIDA" == erro ]] && { echo "HTTP 404" >&2; exit 1; }; echo "${STUB_PROTEGIDA:-false}" ;;
  user) [[ -n "$STUB_LOGIN" ]] && { echo "$STUB_LOGIN"; exit 0; }; echo "sem login" >&2; exit 1 ;;
  *) [[ -n "$STUB_HEAD" ]] && { echo "$STUB_HEAD"; exit 0; }; echo "sem pr" >&2; exit 1 ;;
esac"#,
            );
            rig.stub(
                "devtunnel",
                r#"[[ "$STUB_TUNEL" == erro ]] && { echo "falhou" >&2; exit 1; }; echo "${STUB_TUNEL:-Anonymous  Connect}""#,
            );
            rig
        }

        pub fn p(&self, rel: &str) -> PathBuf {
            self.t.path().join(rel)
        }

        pub fn next(&self) -> usize {
            let n = self.n.get();
            self.n.set(n + 1);
            n
        }

        fn stub(&self, name: &str, text: &str) {
            let f = self.p("bin").join(name);
            fs::write(&f, format!("#!/usr/bin/env bash\n{text}\n")).unwrap();
            fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).unwrap();
        }

        pub fn git(&self, cwd: &Path, args: &[&str]) -> String {
            let o = Command::new("git")
                .arg("-C")
                .arg(cwd)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }

        /// Repositório novo na branch `feature` com um commit; origin opcional (vazio = sem origin).
        pub fn repo(&self, origin: &str) -> PathBuf {
            let d = self.p(&format!("r{}", self.next()));
            fs::create_dir(&d).unwrap();
            self.git(&d, &["init", "-q", "-b", "feature"]);
            self.git(&d, &["config", "user.email", "t@t"]);
            self.git(&d, &["config", "user.name", "t"]);
            if !origin.is_empty() {
                self.git(&d, &["remote", "add", "origin", origin]);
            }
            fs::write(d.join("a.txt"), "um\n").unwrap();
            self.git(&d, &["add", "-A"]);
            self.git(&d, &["commit", "-qm", "um"]);
            d
        }

        pub fn repo_padrao(&self) -> PathBuf {
            self.repo("git@github.com:acme/demo.git")
        }

        /// `run` (`checks-rig.ts:59`): o ambiente fechado do teste mais `env`, o contexto `ctx`.
        pub fn ce(&self, dir: &Path, ctx: &[(&str, &str)], env: &[(&str, &str)]) -> CheckEnv {
            let path = format!(
                "{}:{}",
                self.p("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            );
            let mut e: BTreeMap<String, String> = [
                ("HOME", self.p("home").display().to_string()),
                ("PATH", path),
                ("LANG", "C.UTF-8".into()),
                ("HADES_PACKS_ROOT", self.p("packs").display().to_string()),
                ("HADES_WORK_ROOT", self.p("work").display().to_string()),
                ("HADES_CERBERUS_PORTA_TUNEL_HOSTS", "*.devtunnels.ms".into()),
                ("GIT_CONFIG_GLOBAL", "/dev/null".into()),
                ("GIT_CONFIG_SYSTEM", "/dev/null".into()),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
            for (k, v) in env {
                e.insert((*k).to_string(), (*v).to_string());
            }
            CheckEnv {
                dir: dir.to_path_buf(),
                skill: self.p("skill").display().to_string(),
                momento: "push".into(),
                ctx: ctx
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect::<IndexMap<_, _>>(),
                env: e,
            }
        }
    }
}

use rig::Rig;

fn res(g: Guarda, ce: &CheckEnv) -> String {
    g(ce).to_json()["resultado"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn guarda_remote_push() {
    let r = Rig::new();
    let d = r.repo_padrao();
    r.git(&d, &["checkout", "-q", "--detach"]);
    assert_eq!(
        res(remote_push, &r.ce(&d, &[], &[])),
        "barra",
        "HEAD destacado"
    );
    assert_eq!(
        res(remote_push, &r.ce(&r.repo_padrao(), &[], &[])),
        "aviso",
        "sem remote"
    );
    let d = r.repo_padrao();
    r.git(&d, &["config", "branch.feature.pushRemote", "no_push"]);
    assert_eq!(
        res(remote_push, &r.ce(&d, &[], &[])),
        "barra",
        "pushRemote no_push"
    );
    let d = r.repo_padrao();
    r.git(&d, &["config", "branch.feature.remote", "origin"]);
    r.git(&d, &["remote", "set-url", "--push", "origin", "no_push"]);
    assert_eq!(
        res(remote_push, &r.ce(&d, &[], &[])),
        "barra",
        "pushurl no_push"
    );
    let d = r.repo_padrao();
    r.git(&d, &["config", "branch.feature.pushRemote", "fantasma"]);
    assert_eq!(
        res(remote_push, &r.ce(&d, &[], &[])),
        "barra",
        "remote inexistente"
    );
    let d = r.repo_padrao();
    r.git(&d, &["config", "branch.feature.remote", "origin"]);
    assert_eq!(
        res(remote_push, &r.ce(&d, &[], &[])),
        "ok",
        "remote com url de push"
    );
}

#[test]
fn guarda_upstream_push() {
    let r = Rig::new();
    let com_upstream = |merge: &str| {
        let d = r.repo_padrao();
        r.git(&d, &["config", "branch.feature.remote", "origin"]);
        r.git(
            &d,
            &[
                "config",
                "branch.feature.merge",
                &format!("refs/heads/{merge}"),
            ],
        );
        d
    };
    let g = upstream_push;
    assert_eq!(res(g, &r.ce(&com_upstream("feature"), &[], &[])), "aviso");
    assert_eq!(
        res(
            g,
            &r.ce(&com_upstream("feature"), &[("head", "feature")], &[])
        ),
        "ok"
    );
    assert_eq!(
        res(
            g,
            &r.ce(&com_upstream("outra"), &[("head", "feature")], &[])
        ),
        "barra"
    );
    assert_eq!(
        res(g, &r.ce(&r.repo_padrao(), &[("head", "feature")], &[])),
        "ok"
    );
    assert_eq!(
        res(g, &r.ce(&r.repo_padrao(), &[("head", "x")], &[])),
        "barra"
    );
    let pr = [("pr", "1234")];
    assert_eq!(
        res(g, &r.ce(&r.repo_padrao(), &pr, &[("STUB_HEAD", "feature")])),
        "ok"
    );
    assert_ne!(
        res(g, &r.ce(&r.repo_padrao(), &pr, &[])),
        "ok",
        "gh pr view falha"
    );
    let d = r.repo_padrao();
    r.git(&d, &["checkout", "-q", "--detach"]);
    assert_eq!(res(g, &r.ce(&d, &[("head", "feature")], &[])), "barra");
}

#[test]
fn guarda_branch_protegida() {
    let r = Rig::new();
    let onto = |merge: &str, origin: &str| {
        let d = r.repo(origin);
        r.git(&d, &["config", "branch.feature.remote", "origin"]);
        r.git(
            &d,
            &[
                "config",
                "branch.feature.merge",
                &format!("refs/heads/{merge}"),
            ],
        );
        d
    };
    let o = "git@github.com:acme/demo.git";
    let g = branch_protegida;
    assert_eq!(res(g, &r.ce(&onto("main", o), &[], &[])), "barra");
    assert_eq!(
        res(
            g,
            &r.ce(&onto("release", o), &[], &[("STUB_PROTEGIDA", "true")])
        ),
        "barra"
    );
    assert_eq!(res(g, &r.ce(&onto("feature", o), &[], &[])), "ok");
    assert_eq!(
        res(
            g,
            &r.ce(&onto("feature", o), &[], &[("STUB_PROTEGIDA", "erro")])
        ),
        "aviso"
    );
    assert_eq!(res(g, &r.ce(&onto("feature", "solto"), &[], &[])), "aviso");
    assert_eq!(
        res(g, &r.ce(&r.repo_padrao(), &[("head", "develop")], &[])),
        "barra"
    );
    let d = r.repo_padrao();
    r.git(&d, &["checkout", "-q", "--detach"]);
    assert_eq!(res(g, &r.ce(&d, &[], &[])), "barra");
}

#[test]
fn guarda_conta_gh() {
    let r = Rig::new();
    let perfil = |gh: serde_json::Value| {
        let slug = format!("acme-{}", r.next());
        let dir = r.p("packs").join(&slug);
        fs::create_dir_all(&dir).unwrap();
        let p = serde_json::json!({ "name": slug, "identity": { "repo": format!("acme/{slug}"), "gh": gh } });
        fs::write(dir.join("profile.json"), p.to_string()).unwrap();
        r.repo(&format!("git@github.com:acme/{slug}.git"))
    };
    let g = conta_gh;
    assert_eq!(res(g, &r.ce(&r.repo_padrao(), &[], &[])), "aviso");
    assert_eq!(
        res(
            g,
            &r.ce(
                &perfil(serde_json::json!({"user": "Dev"})),
                &[],
                &[("STUB_LOGIN", "dev")]
            )
        ),
        "ok"
    );
    assert_eq!(
        res(
            g,
            &r.ce(
                &perfil(serde_json::json!({"user": "dev"})),
                &[],
                &[("STUB_LOGIN", "outro")]
            )
        ),
        "barra"
    );
    assert_eq!(
        res(
            g,
            &r.ce(&perfil(serde_json::json!({"user": "dev"})), &[], &[])
        ),
        "aviso"
    );
    assert_eq!(
        res(
            g,
            &r.ce(
                &perfil(serde_json::json!({"tokenEnv": "TOKEN_DO_TESTE"})),
                &[],
                &[]
            )
        ),
        "barra"
    );
    let d = perfil(serde_json::json!({"user": "dev", "tokenEnv": "TOKEN_DO_TESTE"}));
    assert_eq!(
        res(
            g,
            &r.ce(&d, &[], &[("TOKEN_DO_TESTE", "t"), ("STUB_LOGIN", "dev")])
        ),
        "ok"
    );
}

#[test]
fn guarda_nome_dominio() {
    let r = Rig::new();
    let com_script = |body: &str| {
        let d = r.repo_padrao();
        fs::create_dir(d.join("kit")).unwrap();
        let f = d.join("kit").join("skill-release.sh");
        fs::write(&f, format!("#!/usr/bin/env bash\n{body}\n")).unwrap();
        d
    };
    let g = nome_dominio;
    assert_eq!(res(g, &r.ce(&r.repo_padrao(), &[], &[])), "ok");
    assert_eq!(
        res(g, &r.ce(&com_script("echo limpo; exit 0"), &[], &[])),
        "ok"
    );
    assert_eq!(
        res(
            g,
            &r.ce(&com_script("echo 'a.md:1: termo'; exit 1"), &[], &[])
        ),
        "barra"
    );
    assert_eq!(res(g, &r.ce(&com_script("exit 1"), &[], &[])), "barra");
    let sem_perfil = com_script("echo 'guarda de nome: nenhum perfil instalado'");
    assert_eq!(res(g, &r.ce(&sem_perfil, &[], &[])), "aviso");
    assert_eq!(res(g, &r.ce(&com_script("exit 0"), &[], &[])), "ok");
}

#[test]
fn guarda_plano_aprovado() {
    let r = Rig::new();
    let texto = "# Plano\n\n## Objetivo\nx\n\n## Estado atual\nv1\n";
    let plano = r.p("plan.md");
    fs::write(&plano, texto).unwrap();
    let sha = plan_sha(texto);
    let registro = |body: String| {
        let f = r.p(&format!("plan-{}.json", r.next()));
        fs::write(&f, body).unwrap();
        f.display().to_string()
    };
    let reg = |path: &Path, extra: serde_json::Value| {
        let mut v = serde_json::json!({ "path": path.display().to_string(), "sha": sha });
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        registro(v.to_string())
    };
    let g = plano_aprovado;
    let chama = |f: &str| g(&r.ce(&r.repo_padrao(), &[("plano", f)], &[]));
    let resultado = |f: &str| {
        chama(f).to_json()["resultado"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let nada = r.p("nao-existe.json").display().to_string();
    assert_eq!(resultado(&nada), "barra");
    let by = serde_json::json!({ "approvedAt": "2026-01-01T00:00:00Z", "by": { "user": "dono", "origem": "painel" } });
    assert_eq!(resultado(&reg(&plano, by)), "ok");
    let sem_quem = chama(&reg(&plano, serde_json::json!({})));
    assert_eq!(sem_quem.to_json()["resultado"], "ok");
    assert!(sem_quem.detalhe.is_some(), "sem quem aprovou leva detalhe");
    assert_eq!(
        resultado(&reg(&plano, serde_json::json!({ "by": "dono" }))),
        "ok"
    );
    let estado = r.p("plan-estado.md");
    fs::write(
        &estado,
        "# Plano\n\n## Objetivo\nx\n\n## Estado atual (27/10)\nv2\n",
    )
    .unwrap();
    assert_eq!(resultado(&reg(&estado, serde_json::json!({}))), "ok");
    let mudou = r.p("plan-mudou.md");
    fs::write(&mudou, "# Plano\n\n## Objetivo\ny\n").unwrap();
    assert_eq!(resultado(&reg(&mudou, serde_json::json!({}))), "barra");
    assert_eq!(
        resultado(&reg(&r.p("sumiu.md"), serde_json::json!({}))),
        "barra"
    );
    assert_eq!(resultado(&registro("{não é json".into())), "barra");
    let sem_sha = serde_json::json!({ "path": plano.display().to_string() }).to_string();
    assert_eq!(resultado(&registro(sem_sha)), "barra");
    let sem_ctx = g(&r.ce(&r.repo_padrao(), &[], &[]));
    assert_eq!(
        sem_ctx.to_json()["resultado"],
        "barra",
        "sem registro na raiz de trabalho"
    );
}

#[test]
fn guarda_segredo_diff() {
    let r = Rig::new();
    let token_gh = format!("gh{}{}", "p_", "a1B2".repeat(9));
    let chave = format!("-----BEGIN {} {}ATE KEY-----", "RSA", "PRIV");
    let arquivo = |nome: &str, texto: &str| {
        let f = r.p(&format!("{}-{nome}", r.next()));
        fs::write(&f, texto).unwrap();
        f.display().to_string()
    };
    let g = segredo_diff;
    let ctx1 = |k: &'static str, v: String| g(&r.ce(&r.repo_padrao(), &[(k, &v)], &[]));
    let rs = |v: CheckVerdict| v.to_json()["resultado"].as_str().unwrap().to_string();
    assert_eq!(
        rs(ctx1(
            "texto",
            arquivo("c.txt", &format!("olha {token_gh}\n"))
        )),
        "barra"
    );
    assert_eq!(rs(ctx1("arquivo", arquivo("l.txt", "nada aqui\n"))), "ok");
    assert_eq!(
        rs(ctx1("arquivo", r.p("nada").display().to_string())),
        "barra"
    );
    let pasta = r.p(&format!("pasta-{}", r.next()));
    fs::create_dir_all(pasta.join(".git")).unwrap();
    fs::create_dir(pasta.join("sub")).unwrap();
    fs::write(pasta.join(".git").join("x"), format!("{chave}\n")).unwrap();
    fs::write(pasta.join("sub").join("y.txt"), format!("{chave}\n")).unwrap();
    let v = ctx1("arquivo", pasta.display().to_string());
    let onde = v.to_json().to_string();
    assert_eq!(rs(v), "barra");
    assert!(onde.contains("y.txt") && !onde.contains(".git/x"), "{onde}");
    let exemplo = format!("token = \"{}\"\nurl=https://u:<senha>@h\n", "x".repeat(20));
    assert_eq!(rs(ctx1("texto", arquivo("e.txt", &exemplo))), "ok");
    let atrib = format!(
        "api_key: \"{}\"\nhttps://dev:{}@host.io/x\n",
        "Zq9".repeat(8),
        "s3gredo"
    );
    assert_eq!(rs(ctx1("texto", arquivo("a.txt", &atrib))), "barra");

    let d = r.repo("");
    r.git(&d, &["branch", "-m", "solta"]);
    assert_eq!(rs(g(&r.ce(&d, &[], &[]))), "aviso");
    let com_commit = |texto: &str| {
        let d = r.repo_padrao();
        let base = r.git(&d, &["rev-parse", "HEAD"]);
        fs::write(d.join("b.txt"), texto).unwrap();
        r.git(&d, &["add", "-A"]);
        r.git(&d, &["commit", "-qm", "x"]);
        (d, format!("{base}..HEAD"))
    };
    let (d, range) = com_commit(&format!("linha\n{token_gh}\n"));
    let v = g(&r.ce(&d, &[("range", &range)], &[]));
    assert!(
        v.to_json().to_string().contains("b.txt:2"),
        "{}",
        v.to_json()
    );
    assert_eq!(rs(v), "barra");
    let (d, range) = com_commit("limpo\n");
    assert_eq!(rs(g(&r.ce(&d, &[("range", &range)], &[]))), "ok");
    assert_eq!(rs(ctx1("range", "nao..existe".into())), "barra");
    let d = r.repo_padrao();
    r.git(&d, &["tag", "v1"]);
    fs::write(d.join("c.txt"), format!("{token_gh}\n")).unwrap();
    r.git(&d, &["add", "-A"]);
    r.git(&d, &["commit", "-qm", "depois da tag"]);
    assert_eq!(rs(g(&r.ce(&d, &[], &[]))), "barra");
}

#[test]
fn guarda_tunel_acesso() {
    let r = Rig::new();
    let d = r.repo_padrao();
    let t = |ctx: &[(&str, &str)], env: &[(&str, &str)]| {
        tunel_acesso(&r.ce(&d, ctx, env)).to_json()["resultado"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let h = ("host", "a.devtunnels.ms");
    let aut = ("acesso", "autenticado");
    assert_eq!(t(&[], &[]), "barra");
    assert_eq!(
        t(&[h], &[("HADES_CERBERUS_PORTA_TUNEL_HOSTS", "")]),
        "barra"
    );
    assert_eq!(t(&[("host", "evil.example.com"), aut], &[]), "barra");
    assert_eq!(t(&[("host", "https://a.devtunnels.ms/x"), aut], &[]), "ok");
    assert_eq!(t(&[h, ("acesso", "anonimo")], &[]), "barra");
    assert_eq!(t(&[h], &[]), "barra");
    assert_eq!(t(&[h, ("tunel", "t1")], &[]), "barra");
    assert_eq!(
        t(&[h, ("tunel", "t1")], &[("STUB_TUNEL", "Tenant  Connect")]),
        "ok"
    );
    assert_eq!(t(&[h, ("tunel", "t1")], &[("STUB_TUNEL", "erro")]), "aviso");
    let lista = "HADES_CERBERUS_PORTA_TUNEL_HOSTS";
    assert_eq!(
        t(
            &[("host", "a1.devtunnels.ms"), aut],
            &[(lista, "a?.devtunnels.ms")]
        ),
        "ok"
    );
    assert_eq!(t(&[("host", "b.x.io"), aut], &[(lista, "[ab].x.io")]), "ok");
    assert_eq!(
        t(&[("host", "c.x.io"), aut], &[(lista, "[!ab].x.io")]),
        "ok"
    );
    assert_eq!(
        t(&[("host", "a.x.io"), aut], &[(lista, "[!ab].x.io")]),
        "barra"
    );
}
