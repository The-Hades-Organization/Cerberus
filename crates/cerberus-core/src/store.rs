//! Estado fora do repositório (`src/lib/cerberus.ts:146-157` e `378-469`): `<raiz de trabalho>/_maquina/cerberus`
//! com o histórico `vereditos.jsonl` (girado em `vereditos.1.jsonl`), as exceções `excecoes.json` e o `.lock` que
//! serializa quem grava (o mesmo flock do módulo Python).

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use hades_core::config::split_lines;
use hades_core::fsx::{is_file, read_text};
use hades_core::journal::{self, EmitOptions, Source};
use hades_core::jsonfmt::py_dumps;
use hades_core::lock::with_flock;
use hades_core::paths::work_root;
use serde_json::{Map, Value};

use crate::config::{Cfg, cfg_int};
use crate::ctx::parse_ctx;
use crate::modelo::{
    Dict, EXC_VALIDITY_MIN_CHARS, Excecao, Momento, TEXT_MAX, js_norm, js_num, js_string, js_trim,
    now_iso, now_secs, tamanho,
};
use crate::registro::Registro;
use crate::{Erro, Opcoes};

const HISTORICO: &str = "vereditos.jsonl";
const HISTORICO_VELHO: &str = "vereditos.1.jsonl";
const EXCECOES: &str = "excecoes.json";

/// A pasta de estado do Cerberus.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

/// O que `abrir_excecao` recebe além da guarda e do motivo (`OpenException`, `src/lib/cerberus.ts:443`).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct NovaExcecao {
    pub alvo: Vec<(String, String)>,
    /// Quem abriu; ausente, `"dono"`.
    pub por: Option<String>,
    pub origem: String,
    pub ttl_s: i64,
    /// Instante em segundos; ausente, o relógio.
    pub now: Option<f64>,
}

impl NovaExcecao {
    pub fn nova(origem: impl Into<String>, ttl_s: i64) -> Self {
        Self {
            origem: origem.into(),
            ttl_s,
            ..Self::default()
        }
    }
}

impl Store {
    /// `store` (`src/lib/cerberus.ts:153`): `<work_root>/_maquina/cerberus`, criada se falta.
    pub fn abrir(work_root: &Path) -> io::Result<Store> {
        let dir = work_root.join("_maquina").join("cerberus");
        fs::create_dir_all(&dir)?;
        Ok(Store { dir })
    }

    /// A pasta de estado da raiz de trabalho do ambiente (`workRoot`, `src/lib/cerberus.ts:146`).
    pub fn de(op: &Opcoes) -> io::Result<Store> {
        Store::abrir(&work_root(&op.env))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn excecoes_path(&self) -> PathBuf {
        self.dir.join(EXCECOES)
    }

    /// `locked` (`src/lib/cerberus.ts:379`).
    fn travado<T>(&self, f: impl FnOnce() -> T) -> io::Result<T> {
        with_flock(&self.dir.join(".lock"), f)
    }

    /// `appendHistory` (`src/lib/cerberus.ts:394`): o diário primeiro, depois o histórico, girado quando passa de
    /// `HADES_CERBERUS_HIST_KB`.
    pub fn anexar_historico(&self, records: &[Dict], cfg: &Cfg, op: &Opcoes) -> Result<(), Erro> {
        if records.is_empty() {
            return Ok(());
        }
        diario(records, cfg, op);
        let limite = cfg_int(cfg, "HADES_CERBERUS_HIST_KB", 16, 1_048_576)? as u64 * 1024;
        let file = self.dir.join(HISTORICO);
        let texto: String = records
            .iter()
            .map(|r| format!("{}\n", py_dumps(&Value::Object(r.clone()), None)))
            .collect();
        self.travado(|| -> io::Result<()> {
            if is_file(&file) && fs::metadata(&file)?.len() >= limite {
                fs::rename(&file, self.dir.join(HISTORICO_VELHO))?;
            }
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file)?
                .write_all(texto.as_bytes())
        })??;
        Ok(())
    }

    /// `readHistory` (`src/lib/cerberus.ts:405`): o girado e depois o atual. Linha cortada (disco cheio) ou que não é
    /// objeto fica de fora; o resto segue legível.
    pub fn ler_historico(&self) -> Vec<Dict> {
        let mut out = Vec::new();
        for name in [HISTORICO_VELHO, HISTORICO] {
            let Some(text) = read_text(&self.dir.join(name)) else {
                continue;
            };
            for line in split_lines(&text) {
                if let Ok(Value::Object(row)) = serde_json::from_str::<Value>(&line).map(js_norm) {
                    out.push(row);
                }
            }
        }
        out
    }

    /// `loadExceptions` (`src/lib/cerberus.ts:424`): ausente é lista vazia; ilegível é recusa (quem regravasse
    /// perderia as abertas); o que não é lista vale vazio.
    pub fn carregar_excecoes(&self) -> Result<Vec<Excecao>, Erro> {
        let path = self.excecoes_path();
        let Some(text) = read_text(&path) else {
            return Ok(Vec::new());
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Array(items)) => Ok(items.into_iter().map(|v| Excecao(js_norm(v))).collect()),
            Ok(_) => Ok(Vec::new()),
            Err(_) => Err(Erro::recusa(format!(
                "{} ilegível: conserte ou apague (as exceções abertas se perdem)",
                path.display()
            ))),
        }
    }

    /// `saveExceptions` (`src/lib/cerberus.ts:436`): `excecoes.tmp` e `rename`, recuo 1 como o Python.
    pub fn gravar_excecoes(&self, list: &[Excecao]) -> io::Result<()> {
        let tmp = self.dir.join("excecoes.tmp");
        let value = Value::Array(list.iter().map(|e| e.0.clone()).collect());
        fs::write(&tmp, py_dumps(&value, Some(1)))?;
        fs::rename(&tmp, self.excecoes_path())
    }

    /// `openException` (`src/lib/cerberus.ts:451`): grava uma exceção de uso único. Quem chama isto é só o dono.
    pub fn abrir_excecao(
        &self,
        registro: &Registro,
        cab: &str,
        id: &str,
        momento: &str,
        motivo: &str,
        opts: NovaExcecao,
    ) -> Result<Excecao, Erro> {
        let now = opts.now.unwrap_or_else(now_secs);
        if registro.achar(cab, id).is_none() {
            return Err(Erro::recusa(format!("guarda {cab}/{id} não registrada")));
        }
        if Momento::parse(momento).is_none() {
            return Err(Erro::recusa(format!("momento inválido: {momento}")));
        }
        if tamanho(js_trim(motivo)) < EXC_VALIDITY_MIN_CHARS || tamanho(motivo) > TEXT_MAX {
            return Err(Erro::recusa(format!(
                "motivo obrigatório ({EXC_VALIDITY_MIN_CHARS} a {TEXT_MAX} caracteres)"
            )));
        }
        let pares: Vec<String> = opts.alvo.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let alvo: Map<String, Value> = parse_ctx(&pares)?
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        let mut exc = Map::new();
        exc.insert("id".into(), Value::String(id_aleatorio()?));
        exc.insert("cabeca".into(), cab.into());
        exc.insert("guarda".into(), id.into());
        exc.insert("momento".into(), momento.into());
        exc.insert("alvo".into(), Value::Object(alvo));
        exc.insert("motivo".into(), js_trim(motivo).into());
        exc.insert("por".into(), opts.por.as_deref().unwrap_or("dono").into());
        exc.insert("origem".into(), opts.origem.into());
        exc.insert("aberta".into(), now_iso(now).into());
        exc.insert("abertaTs".into(), js_num(now));
        exc.insert("expira".into(), js_num(now + opts.ttl_s as f64));
        exc.insert("usada".into(), Value::Null);
        let exc = Excecao(Value::Object(exc));
        self.travado(|| -> Result<(), Erro> {
            let mut list = self.carregar_excecoes()?;
            list.push(exc.clone());
            self.gravar_excecoes(&list)?;
            Ok(())
        })??;
        Ok(exc)
    }
}

/// `randomBytes(6).toString("hex")`.
fn id_aleatorio() -> io::Result<String> {
    let mut bytes = [0u8; 6];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// `journalVerdicts` (`src/lib/cerberus.ts:384`): só o que barra ou avisa vai ao diário; o ok é a maioria e não diz
/// nada ao Argos.
fn diario(records: &[Dict], cfg: &Cfg, op: &Opcoes) {
    for r in records {
        let efeito = r.get("efeito").and_then(Value::as_str);
        if !matches!(efeito, Some("barra" | "aviso")) {
            continue;
        }
        let proof = r
            .get("evidencia")
            .and_then(Value::as_array)
            .and_then(|ev| ev.first())
            .and_then(Value::as_object)
            .map(|e| match e.get("conferido") {
                None | Some(Value::Null) => String::new(),
                v => js_string(v),
            })
            .unwrap_or_default();
        let mut summary = format!(
            "{}/{} {}",
            js_string(r.get("cabeca")),
            js_string(r.get("guarda")),
            js_string(r.get("momento"))
        );
        if !proof.is_empty() {
            summary.push_str(&format!(": {proof}"));
        }
        let cwd = r
            .get("dir")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| op.cwd.to_string_lossy().into_owned());
        let get = |key: &str| cfg.get(key).cloned();
        journal::emit(
            Source::Cerberus,
            &format!("cerberus.{}", efeito.unwrap_or_default()),
            &summary,
            EmitOptions {
                cwd: Some(cwd),
                settings: Some(journal::journal_settings(&get)),
                env: Some(&op.env),
                ..EmitOptions::default()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testes::Caixa;
    use serde_json::json;

    const GUARDA: &str =
        r#"{"titulo": "T", "origem": "o", "momentos": ["push"], "tipo": "emitida"}"#;

    fn cfg(hist_kb: &str) -> Cfg {
        [("HADES_CERBERUS_HIST_KB", hist_kb), ("HADES_JOURNAL", "0")]
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn registro(dir: &Value) -> Dict {
        let mut r = json!({
            "ts": "2025-10-26T12:00:00Z", "avaliacao": "a1", "cabeca": "vigilia", "guarda": "g", "momento": "continuo",
            "resultado": "barra", "evidencia": [{"conferido": "ação", "onde": "b"}], "efeito": "barra",
            "modo": "avisa", "duracaoS": 0.25, "n": 3,
        });
        r["dir"] = dir.clone();
        r.as_object().unwrap().clone()
    }

    #[test]
    fn historico_byte_a_byte_e_leitura() {
        let caixa = Caixa::nova();
        let op = caixa.opcoes();
        let store = Store::de(op).unwrap();
        assert_eq!(store.dir(), caixa.raiz().join("work/_maquina/cerberus"));
        store.anexar_historico(&[], &cfg("x"), op).unwrap();
        assert!(!store.dir().join(HISTORICO).exists());

        let r = registro(&json!("/tmp"));
        store
            .anexar_historico(&[r.clone(), r], &cfg("16"), op)
            .unwrap();
        let linha = r#"{"ts": "2025-10-26T12:00:00Z", "avaliacao": "a1", "cabeca": "vigilia", "guarda": "g", "momento": "continuo", "resultado": "barra", "evidencia": [{"conferido": "ação", "onde": "b"}], "efeito": "barra", "modo": "avisa", "duracaoS": 0.25, "n": 3, "dir": "/tmp"}"#;
        let texto = fs::read_to_string(store.dir().join(HISTORICO)).unwrap();
        assert_eq!(texto, format!("{linha}\n{linha}\n"));

        fs::write(
            store.dir().join(HISTORICO_VELHO),
            "{\"a\": 1.0}\n[1]\n{\"cort",
        )
        .unwrap();
        let lidos = store.ler_historico();
        assert_eq!(lidos.len(), 3);
        assert_eq!(lidos[0], *json!({"a": 1}).as_object().unwrap());
        assert_eq!(lidos[1]["n"], json!(3));
    }

    #[test]
    fn historico_gira_no_limite() {
        let caixa = Caixa::nova();
        let op = caixa.opcoes();
        let store = Store::de(op).unwrap();
        assert_eq!(
            store
                .anexar_historico(&[registro(&json!(null))], &cfg("15"), op)
                .unwrap_err()
                .to_string(),
            "HADES_CERBERUS_HIST_KB=15 fora de 16–1048576"
        );
        let atual = store.dir().join(HISTORICO);
        fs::write(&atual, format!("{}\n", "x".repeat(16 * 1024 - 2))).unwrap();
        store
            .anexar_historico(&[registro(&json!(null))], &cfg("16"), op)
            .unwrap();
        assert!(!store.dir().join(HISTORICO_VELHO).exists());
        assert!(fs::metadata(&atual).unwrap().len() >= 16 * 1024);
        store
            .anexar_historico(&[registro(&json!(null))], &cfg("16"), op)
            .unwrap();
        assert_eq!(
            fs::metadata(store.dir().join(HISTORICO_VELHO))
                .unwrap()
                .len() as usize,
            {
                let um = fs::read_to_string(&atual).unwrap().len();
                16 * 1024 - 1 + um
            }
        );
        assert_eq!(store.ler_historico().len(), 2);
    }

    #[test]
    fn diario_recebe_so_barra_e_aviso() {
        let caixa = Caixa::nova();
        let op = caixa.opcoes();
        let store = Store::de(op).unwrap();
        let mut ok = registro(&json!(null));
        ok.insert("efeito".into(), json!("ok"));
        let mut c = cfg("16");
        c.shift_remove("HADES_JOURNAL");
        store
            .anexar_historico(&[ok, registro(&json!(null))], &c, op)
            .unwrap();
        let pasta = caixa.raiz().join("work/_maquina/journal");
        let mut linhas = Vec::new();
        let mut pilha = vec![pasta];
        while let Some(d) = pilha.pop() {
            for e in fs::read_dir(&d).into_iter().flatten().flatten() {
                if e.path().is_dir() {
                    pilha.push(e.path());
                } else {
                    linhas.extend(
                        fs::read_to_string(e.path())
                            .unwrap()
                            .lines()
                            .map(str::to_owned),
                    );
                }
            }
        }
        let eventos: Vec<_> = linhas.iter().filter_map(|l| journal::parse(l)).collect();
        let resumos: Vec<(&str, &str)> = eventos
            .iter()
            .map(|e| (e.kind.as_str(), e.summary.as_str()))
            .collect();
        assert_eq!(resumos, [("cerberus.barra", "vigilia/g continuo: ação")]);
    }

    #[test]
    fn excecoes_abrir_gravar_e_ler() {
        let caixa = Caixa::nova();
        caixa.guarda("porta", "t-b", GUARDA);
        let op = caixa.opcoes();
        let reg = Registro::carregar(&op.skill());
        let store = Store::de(op).unwrap();
        assert!(store.carregar_excecoes().unwrap().is_empty());

        let mut opts = NovaExcecao::nova("painel", 3600);
        opts.alvo = vec![("pr".into(), "1".into())];
        opts.now = Some(1_761_480_000.0);
        let exc = store
            .abrir_excecao(
                &reg,
                "porta",
                "t-b",
                "push",
                "  motivo de teste ",
                opts.clone(),
            )
            .unwrap();
        let id = exc.id().unwrap().to_owned();
        assert_eq!(id.len(), 12);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
        let esperado = format!(
            "[\n {{\n  \"id\": \"{id}\",\n  \"cabeca\": \"porta\",\n  \"guarda\": \"t-b\",\n  \"momento\": \"push\",\n  \"alvo\": {{\n   \"pr\": \"1\"\n  }},\n  \"motivo\": \"motivo de teste\",\n  \"por\": \"dono\",\n  \"origem\": \"painel\",\n  \"aberta\": \"2025-10-26T12:00:00Z\",\n  \"abertaTs\": 1761480000,\n  \"expira\": 1761483600,\n  \"usada\": null\n }}\n]"
        );
        assert_eq!(fs::read_to_string(store.excecoes_path()).unwrap(), esperado);
        assert_eq!(
            store.carregar_excecoes().unwrap(),
            std::slice::from_ref(&exc)
        );

        opts.now = Some(1_761_480_000.5);
        let outra = store
            .abrir_excecao(&reg, "porta", "t-b", "push", "outro motivo bom", opts)
            .unwrap();
        assert_eq!(outra.campos().unwrap()["expira"], json!(1761483600.5));
        assert_eq!(store.carregar_excecoes().unwrap().len(), 2);
    }

    #[test]
    fn abrir_excecao_recusa_na_ordem_do_ts() {
        let caixa = Caixa::nova();
        caixa.guarda("porta", "t-b", GUARDA);
        let op = caixa.opcoes();
        let reg = Registro::carregar(&op.skill());
        let store = Store::de(op).unwrap();
        let abrir = |cab: &str, id: &str, m: &str, motivo: &str, alvo: Vec<(String, String)>| {
            let mut o = NovaExcecao::nova("painel", 60);
            o.alvo = alvo;
            store
                .abrir_excecao(&reg, cab, id, m, motivo, o)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            abrir("porta", "nao", "x", "", vec![]),
            "guarda porta/nao não registrada"
        );
        assert_eq!(
            abrir("porta", "t-b", "x", "", vec![]),
            "momento inválido: x"
        );
        assert_eq!(
            abrir("porta", "t-b", "push", "  curto   ", vec![]),
            "motivo obrigatório (10 a 500 caracteres)"
        );
        assert_eq!(
            abrir("porta", "t-b", "push", &"m".repeat(501), vec![]),
            "motivo obrigatório (10 a 500 caracteres)"
        );
        assert_eq!(
            abrir(
                "porta",
                "t-b",
                "push",
                "motivo de teste",
                vec![("PR".into(), "1".into())]
            ),
            "--ctx 'PR=1': chave [a-z][a-z0-9_]{0,31}=valor de até 300 caracteres"
        );
        assert!(!store.excecoes_path().exists());
    }

    #[test]
    fn excecoes_ilegivel_e_nao_lista() {
        let caixa = Caixa::nova();
        let store = Store::de(caixa.opcoes()).unwrap();
        fs::write(store.excecoes_path(), "{\"a\": 1}").unwrap();
        assert!(store.carregar_excecoes().unwrap().is_empty());
        fs::write(store.excecoes_path(), "[1, {\"expira\": 2.0}]").unwrap();
        assert_eq!(
            store.carregar_excecoes().unwrap(),
            [Excecao(json!(1)), Excecao(json!({"expira": 2}))]
        );
        fs::write(store.excecoes_path(), "[{\"cort").unwrap();
        assert_eq!(
            store.carregar_excecoes().unwrap_err().to_string(),
            format!(
                "{} ilegível: conserte ou apague (as exceções abertas se perdem)",
                store.excecoes_path().display()
            )
        );
        store.gravar_excecoes(&[Excecao(json!(1))]).unwrap();
        assert_eq!(
            fs::read_to_string(store.excecoes_path()).unwrap(),
            "[\n 1\n]"
        );
        assert!(!store.dir().join("excecoes.tmp").exists());
    }
}
