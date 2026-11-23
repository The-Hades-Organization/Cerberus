use super::*;

#[test]
fn dirname_como_o_node() {
    assert_eq!(dirname("/a/b/lim"), "/a/b");
    assert_eq!(dirname("/a/b/lim/"), "/a/b");
    assert_eq!(dirname("lim"), ".");
    assert_eq!(dirname("/lim"), "/");
}

#[test]
fn varredura_de_script() {
    let (mut bad, mut warn) = (Vec::new(), Vec::new());
    scan_script(
        "cp x ~/.claude/skills/hades/loop/hooks/hades-guard.py\njq . ~/.claude/settings.json\n# rm skills/hades\n",
        "env/post-install.sh",
        &mut bad,
        &mut warn,
    );
    let textos: Vec<&str> = bad.iter().map(|e| e.conferido.as_str()).collect();
    assert!(
        textos
            .iter()
            .any(|t| t.starts_with("mexe no hook hades-guard"))
    );
    assert!(
        textos
            .iter()
            .any(|t| t.starts_with("escreve na pasta da skill"))
    );
    assert!(
        textos
            .iter()
            .any(|t| t.contains("(onde o hook é registrado)"))
    );
    assert!(warn.is_empty());
    let (mut bad, mut warn) = (Vec::new(), Vec::new());
    scan_script(
        "echo HADES_SKILL_DIRX; scp a b\n",
        "x.sh",
        &mut bad,
        &mut warn,
    );
    assert!(bad.is_empty() && warn.is_empty());
}

#[test]
fn contexto_incompleto_barra_e_diz_o_que_faltou() {
    let ce = CheckEnv::default();
    let v = rodar("commit-fixado", &ce).unwrap();
    assert_eq!(v.resultado, Barra);
    assert!(v.evidencia[0].conferido.contains("plugin, commit, op"));
    assert_eq!(rodar("ensaio", &ce).unwrap().resultado, Barra);
    assert!(rodar("nada", &ce).is_none());
}

mod rig {
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use indexmap::IndexMap;
    use sha2::{Digest, Sha256};

    use crate::check::CheckEnv;

    pub const ID: &str = "20260101-000000-lim";

    /// Pacote `lim` publicado num remote local, com a prévia do `add` em `plugins-staging/<ID>/` e a instalação em
    /// `packs/lim` (`loop/test/ts/cerberus-limiar.test.ts:14`).
    pub struct Rig {
        pub t: tempfile::TempDir,
        pub c1: String,
        n: Cell<usize>,
    }

    impl Rig {
        pub fn new() -> Self {
            let t = tempfile::tempdir().unwrap();
            let mut rig = Self {
                t,
                c1: String::new(),
                n: Cell::new(0),
            };
            for d in [
                "home",
                "packs",
                "data/logs",
                "skill/kit",
                "skill/loop/hooks",
                "src/env/panel/ui",
            ] {
                fs::create_dir_all(rig.p(d)).unwrap();
            }
            fs::write(rig.p("skill/kit/config.sh"), "echo nucleo\n").unwrap();
            fs::write(rig.p("skill/loop/hooks/guard-rules.json"), "{}\n").unwrap();
            let remote = rig.p("lim.git");
            rig.git(
                rig.t.path(),
                &["init", "-q", "--bare", "-b", "main", &rig.s(&remote)],
            );
            let src = rig.p("src");
            rig.git(&src, &["init", "-q", "-b", "main"]);
            rig.git(&src, &["config", "user.email", "t@t"]);
            rig.git(&src, &["config", "user.name", "t"]);
            rig.git(&src, &["remote", "add", "origin", &rig.s(&remote)]);
            rig.w(
                "src/profile.json",
                "{\"name\": \"lim\", \"identity\": {\"repo\": \"acme/lim\"}, \"env\": {\"pack\": \"env\"}}\n",
            );
            rig.w(
                "src/env/post-install.sh",
                "#!/usr/bin/env bash\ntouch \"$HADES_DATA/lim.ran\"\n",
            );
            rig.w("src/env/lim-svc.sh", "#!/usr/bin/env bash\necho svc\n");
            rig.w(
                "src/env/config.tsv",
                "LIM_LANES\t3\tint:1:9\tProjeto LIM\tFaixas\tnext\tChave do plugin.\n",
            );
            rig.w(
                "src/env/panel/api.ts",
                "export function eco() { return { ok: true }; }\n",
            );
            rig.w(
                "src/env/panel/ui/lim.js",
                "window.HadesPanel.registerPage(\"lim\", () => null);\n",
            );
            rig.w(
                "src/env/plugin.json",
                "{\"name\":\"lim\",\"panel\":{\"pages\":[{\"id\":\"lim\",\"label\":\"LIM\",\"script\":\"lim.js\"}],\"routes\":[{\"path\":\"eco\",\"fn\":\"eco\"}]}}\n",
            );
            rig.git(&src, &["add", "-A"]);
            rig.git(&src, &["commit", "-qm", "v1"]);
            rig.git(&src, &["push", "-q", "origin", "main"]);
            rig.c1 = rig.git(&src, &["rev-parse", "HEAD"]);
            let stage = rig.p(&format!("data/plugins-staging/{ID}"));
            fs::create_dir_all(&stage).unwrap();
            rig.clone_em(&stage.join("lim"));
            fs::write(stage.join("preview.json"), rig.previa(&[])).unwrap();
            rig.clone_em(&rig.p("packs/lim"));
            rig.log(&format!("op=add plugin=lim origem=t commit={}", rig.c1));
            rig
        }

        pub fn p(&self, rel: &str) -> PathBuf {
            self.t.path().join(rel)
        }

        pub fn s(&self, p: &Path) -> String {
            p.display().to_string()
        }

        pub fn w(&self, rel: &str, text: &str) {
            let f = self.p(rel);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, text).unwrap();
        }

        pub fn append(&self, rel: &str, text: &str) {
            let old = fs::read_to_string(self.p(rel)).unwrap_or_default();
            fs::write(self.p(rel), old + text).unwrap();
        }

        pub fn log(&self, line: &str) {
            let n = self.n.get();
            self.n.set(n + 1);
            self.append(
                "data/logs/plugins.log",
                &format!("2026-01-01T00:00:{n:02}Z {line}\n"),
            );
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

        fn clone_em(&self, dest: &Path) {
            self.git(
                self.t.path(),
                &["clone", "-q", &self.s(&self.p("lim.git")), &self.s(dest)],
            );
            self.git(dest, &["config", "user.email", "t@t"]);
            self.git(dest, &["config", "user.name", "t"]);
        }

        fn sha(&self, rel: &str) -> String {
            let body = fs::read(self.p("src").join(rel)).unwrap();
            Sha256::digest(&body)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        }

        /// O `preview.json` que o `plugins add` grava; `troca` sobrescreve o sha de um caminho (`""` tira o caminho).
        pub fn previa(&self, troca: &[(&str, &str)]) -> String {
            let runs: Vec<serde_json::Value> = [
                "env/post-install.sh",
                "env/panel/api.ts",
                "env/panel/ui/lim.js",
            ]
            .into_iter()
            .filter_map(|path| {
                let sha = troca
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map_or_else(|| self.sha(path), |(_, s)| (*s).to_string());
                (!sha.is_empty()).then(|| serde_json::json!({"path": path, "sha256": sha}))
            })
            .collect();
            serde_json::json!({
                "name": "lim",
                "commit": self.c1,
                "runs": runs,
                "other": ["env/lim-svc.sh"],
                "adds": {"pages": [{"id": "lim"}], "routes": ["GET eco"]},
            })
            .to_string()
        }

        pub fn ce(&self, ctx: &[(&str, &str)], data: Option<&Path>) -> CheckEnv {
            let data = data.map_or_else(|| self.p("data"), Path::to_path_buf);
            let env: BTreeMap<String, String> = [
                ("HOME", self.s(&self.p("home"))),
                ("PATH", std::env::var("PATH").unwrap_or_default()),
                ("LANG", "C.UTF-8".into()),
                ("HADES_PACKS_ROOT", self.s(&self.p("packs"))),
                ("HADES_DATA", self.s(&data)),
                ("GIT_CONFIG_GLOBAL", "/dev/null".into()),
                ("GIT_CONFIG_SYSTEM", "/dev/null".into()),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
            CheckEnv {
                dir: self.t.path().to_path_buf(),
                skill: self.s(&self.p("skill")),
                momento: "plugin".into(),
                ctx: ctx
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect::<IndexMap<_, _>>(),
                env,
            }
        }
    }
}

use rig::{ID, Rig};

/// `[commit-fixado, executavel, invariante]` como texto, mais as evidências de cada uma.
fn av(r: &Rig, commit: &str, op: &str, extra: &[(&str, &str)]) -> (String, Vec<String>) {
    let mut ctx = vec![("plugin", "lim"), ("commit", commit), ("op", op)];
    ctx.extend_from_slice(extra);
    av_ce(&r.ce(&ctx, None))
}

fn av_ce(ce: &CheckEnv) -> (String, Vec<String>) {
    let mut res = Vec::new();
    let mut evid = Vec::new();
    for g in ["commit-fixado", "executavel", "invariante"] {
        let v = rodar(g, ce).unwrap();
        res.push(v.resultado.as_str());
        evid.push(
            v.evidencia
                .iter()
                .map(|e| e.conferido.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        );
    }
    (res.join(" "), evid)
}

#[test]
fn add_confere_a_previa() {
    let r = Rig::new();
    let c1 = r.c1.clone();
    let (res, ev) = av(&r, &c1, "add", &[]);
    assert_eq!(res, "ok ok ok", "{ev:?}");
    assert!(ev[0].contains(&format!("commit da prévia = {}", &c1[..12])));
    assert!(ev[1].contains("rotas: ['GET eco']"), "{}", ev[1]);

    let (res, ev) = av(&r, "main", "add", &[]);
    assert!(res.starts_with("barra"));
    assert!(ev[0].contains("não é um sha completo"));
    assert!(av(&r, &c1[..12], "add", &[]).0.starts_with("barra"));
    let (res, ev) = av(&r, &"0".repeat(40), "add", &[]);
    assert!(res.starts_with("barra"));
    assert!(ev[0].contains("nenhuma prévia de lim"));

    let prev = r.p(&format!("data/plugins-staging/{ID}/preview.json"));
    std::fs::write(&prev, r.previa(&[("env/post-install.sh", "00")])).unwrap();
    let (res, ev) = av(&r, &c1, "add", &[]);
    assert_eq!(res.split(' ').nth(1), Some("barra"));
    assert!(ev[1].contains("executável diferente da prévia: env/post-install.sh"));
    std::fs::write(&prev, r.previa(&[("env/panel/api.ts", "")])).unwrap();
    let (res, ev) = av(&r, &c1, "add", &[]);
    assert_eq!(res.split(' ').nth(1), Some("barra"));
    assert!(ev[1].contains("env/panel/api.ts"), "{}", ev[1]);
    std::fs::write(&prev, r.previa(&[])).unwrap();

    let clone = format!("data/plugins-staging/{ID}/lim");
    r.append(&format!("{clone}/env/lim-svc.sh"), "# solto\n");
    assert!(av(&r, &c1, "add", &[]).0.starts_with("barra"));
    r.git(&r.p(&clone), &["checkout", "-q", "--", "."]);

    let pack_dir = r.s(&r.p(&clone));
    assert_eq!(av(&r, &c1, "add", &[("pack_dir", &pack_dir)]).0, "ok ok ok");
    let sem = r.p("solto/lim");
    std::fs::create_dir_all(&sem).unwrap();
    let (res, ev) = av(&r, &c1, "add", &[("pack_dir", &r.s(&sem))]);
    assert!(res.starts_with("barra"));
    assert!(ev[0].contains("nenhuma prévia de lim"));
}

#[test]
fn add_com_script_fora_e_previa_sem_sha() {
    let r = Rig::new();
    let c1 = r.c1.clone();
    let clone = r.p(&format!("data/plugins-staging/{ID}/lim"));
    let prev = r.p(&format!("data/plugins-staging/{ID}/preview.json"));
    let semsha = r.previa(&[]).replace("\"sha256\"", "\"x\"");
    std::fs::write(&prev, semsha).unwrap();
    let (res, ev) = av(&r, &c1, "add", &[]);
    assert_eq!(res.split(' ').nth(1), Some("barra"));
    assert!(
        ev[1].contains("não consegui conferir: 'sha256'"),
        "{}",
        ev[1]
    );

    std::fs::write(r.p("src/env/extra.sh"), "echo x\n").unwrap();
    let pj = r.p("src/env/plugin.json").to_string_lossy().into_owned();
    std::fs::write(
        &pj,
        "{\"name\":\"lim\",\"panel\":{\"pages\":[{\"id\":\"lim\"},{\"id\":\"outra\"}],\"routes\":[{\"path\":\"eco\"}]}}\n",
    )
    .unwrap();
    let src = r.p("src");
    r.git(&src, &["add", "-A"]);
    r.git(&src, &["commit", "-qm", "v1b"]);
    r.git(&src, &["push", "-q", "origin", "main"]);
    let c = r.git(&src, &["rev-parse", "HEAD"]);
    r.git(&clone, &["pull", "-q"]);
    std::fs::write(&prev, r.previa(&[]).replace(&c1, &c)).unwrap();
    let (res, ev) = av(&r, &c, "add", &[]);
    assert_eq!(res, "ok barra ok", "{ev:?}");
    let det = rodar(
        "executavel",
        &r.ce(&[("plugin", "lim"), ("commit", &c), ("op", "add")], None),
    )
    .unwrap()
    .detalhe
    .unwrap();
    assert!(det.contains("script fora da prévia: env/extra.sh"), "{det}");
    assert!(
        det.contains("páginas ou rotas do plugin.json diferentes da prévia: outra"),
        "{det}"
    );
}

#[test]
fn enable_confere_a_ultima_instalacao() {
    let r = Rig::new();
    let c1 = r.c1.clone();
    assert_eq!(av(&r, &c1, "enable", &[]).0, "ok ok ok");
    let pack = r.p("packs/lim");
    r.append("packs/lim/env/lim-svc.sh", "# à mão\n");
    assert!(av(&r, &c1, "enable", &[]).0.starts_with("barra"));
    r.git(&pack, &["checkout", "-q", "--", "."]);
    r.git(&pack, &["commit", "-q", "--allow-empty", "-m", "local"]);
    let local = r.git(&pack, &["rev-parse", "HEAD"]);
    let (res, ev) = av(&r, &local, "enable", &[]);
    assert!(res.starts_with("barra"));
    assert!(ev[0].contains(&format!("último commit instalado no log = {}", &c1[..12])));
    r.git(&pack, &["reset", "-q", "--hard", &c1]);
    let vazio = r.p("sem-log");
    let ce = r.ce(
        &[("plugin", "lim"), ("commit", &c1), ("op", "enable")],
        Some(&vazio),
    );
    assert!(av_ce(&ce).0.starts_with("aviso"));
}

#[test]
fn update_confere_o_que_muda() {
    let r = Rig::new();
    let src = r.p("src");
    let pack = r.p("packs/lim");
    r.append("src/env/post-install.sh", "echo v2\n");
    r.git(&src, &["commit", "-qam", "v2"]);
    r.git(&src, &["push", "-q", "origin", "main"]);
    let c2 = r.git(&src, &["rev-parse", "HEAD"]);
    r.git(&pack, &["fetch", "-q", "origin"]);
    r.log(&format!("op=update-preview plugin=lim target={c2}"));
    let (res, ev) = av(&r, &c2, "update", &[]);
    assert_eq!(res, "ok ok ok", "{ev:?}");
    assert!(ev[1].contains("alterado: post-install.sh"), "{}", ev[1]);

    let ramo = |b: &str, f: &dyn Fn()| {
        f();
        r.git(&src, &["add", "-A"]);
        r.git(&src, &["commit", "-qm", b]);
        r.git(&src, &["push", "-q", "origin", &format!("HEAD:{b}")]);
        r.git(&src, &["reset", "-q", "--hard", &c2]);
        r.git(&src, &["clean", "-qfd"]);
    };
    ramo("b-novo", &|| {
        r.w("src/env/extra.sh", "#!/usr/bin/env bash\n")
    });
    ramo("b-mod", &|| r.append("src/env/lim-svc.sh", "# v2\n"));
    ramo("b-nucleo", &|| r.w("src/env/config.sh", "echo ok\n"));
    ramo("b-guard", &|| {
        r.append(
            "src/env/post-install.sh",
            "cp x ~/.claude/skills/hades/loop/hooks/hades-guard.py\n",
        )
    });
    ramo("b-modo", &|| {
        r.append(
            "src/env/config.tsv",
            "HADES_CERBERUS_PORTA\toff\tenum:off,barra\tG\tL\tnext\tD\n",
        )
    });
    ramo("b-settings", &|| {
        r.append("src/env/post-install.sh", "jq . ~/.claude/settings.json\n")
    });
    ramo("b-settings-hook", &|| {
        r.append(
            "src/env/post-install.sh",
            "jq \".hooks = {}\" ~/.claude/settings.json\n",
        )
    });
    ramo("b-loop", &|| r.w("src/loop/hooks/guard-rules.json", "{}\n"));
    ramo("b-cerberus", &|| r.w("src/env/cerberus-x.sh", "echo oi\n"));
    ramo("b-neutro", &|| r.w("src/env/README.txt", "ok\n"));
    r.git(&pack, &["fetch", "-q", "origin"]);
    let tip = |b: &str| r.git(&pack, &["rev-parse", &format!("origin/{b}")]);
    let un = |b: &str| av(&r, &tip(b), "update", &[]);

    assert_eq!(un("b-neutro").0, "barra ok ok");
    let (res, ev) = un("b-novo");
    assert_eq!(res.split(' ').nth(1), Some("barra"));
    assert!(ev[1].contains("novo: extra.sh (fora do diff da prévia)"));
    assert_eq!(un("b-mod").0.split(' ').nth(1), Some("aviso"));
    let inv = |b: &str| {
        let (res, ev) = un(b);
        (res.split(' ').nth(2).unwrap().to_string(), ev[2].clone())
    };
    let (res, ev) = inv("b-nucleo");
    assert_eq!(res, "barra");
    assert!(
        ev.contains("env/config.sh cai por cima de kit/config.sh"),
        "{ev}"
    );
    let (res, ev) = inv("b-guard");
    assert_eq!(res, "barra");
    assert!(ev.contains("escreve na pasta da skill"), "{ev}");
    let (res, ev) = inv("b-modo");
    assert_eq!(res, "barra");
    assert!(ev.contains("declara HADES_CERBERUS_PORTA"), "{ev}");
    assert_eq!(inv("b-settings").0, "aviso");
    assert_eq!(inv("b-settings-hook").0, "barra");
    let (res, ev) = inv("b-loop");
    assert_eq!(res, "barra");
    assert!(ev.contains("arquivo que já é do núcleo (loop/): substituiria o núcleo"));
    assert_eq!(inv("b-cerberus").0, "barra");

    let ce = r.ce(
        &[("plugin", "inexistente"), ("commit", &c2), ("op", "update")],
        None,
    );
    let (res, ev) = av_ce(&ce);
    assert_eq!(res, "barra barra barra");
    assert!(ev[2].contains("não consegui conferir"));
}
