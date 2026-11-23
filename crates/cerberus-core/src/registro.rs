//! Registro de guardas (`src/lib/cerberus.ts:165-242`), lido de `<skill>/loop/cerberus/<cabeça>/<id>.json` a cada
//! chamada. Nada do registro é embutido no binário: a skill é a fonte.

use std::fs;
use std::path::{Path, PathBuf};

use hades_core::fsx::{is_dir, is_file, list_dir, realpath_missing_ok};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::modelo::{Cabeca, Dict, Momento, TIMEOUT_MAX, Tipo, em_branco};

/// `cmd[0]` de uma guarda exec que roda a CLI da skill (`hades <subcomando>`) em vez de um arquivo da cabeça.
pub const CLI_CMD: &str = "hades";

/// `REGISTRY` (`src/lib/cerberus.ts:166`).
pub fn pasta(skill: &Path) -> PathBuf {
    skill.join("loop").join("cerberus")
}

/// Guarda registrada: o objeto do arquivo mais `cabeca`, `id` e `dono` (`src/lib/cerberus.ts:66`). Os campos tipados
/// são lidos de [`Guarda::campos`], que guarda o objeto inteiro na ordem em que o TS o devolve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Dict", into = "Dict")]
#[non_exhaustive]
pub struct Guarda {
    pub cabeca: Cabeca,
    pub id: String,
    pub tipo: Tipo,
    pub momentos: Vec<Momento>,
    pub titulo: String,
    pub dono: Cabeca,
    /// Só nas guardas exec.
    pub cmd: Vec<String>,
    pub timeout_s: Option<u64>,
    pub campos: Dict,
}

impl Guarda {
    pub fn texto(&self, chave: &str) -> Option<&str> {
        self.campos.get(chave).and_then(Value::as_str)
    }
}

impl TryFrom<Dict> for Guarda {
    type Error = String;

    fn try_from(campos: Dict) -> Result<Self, String> {
        let texto = |k: &str| campos.get(k).and_then(Value::as_str).map(str::to_owned);
        let cabeca = texto("cabeca")
            .and_then(|c| Cabeca::parse(&c))
            .ok_or("cabeca inválida")?;
        let tipo = texto("tipo")
            .and_then(|t| Tipo::parse(&t))
            .ok_or("tipo inválido")?;
        let momentos = campos
            .get("momentos")
            .and_then(Value::as_array)
            .ok_or("momentos ausente")?
            .iter()
            .map(|m| {
                m.as_str()
                    .and_then(Momento::parse)
                    .ok_or("momento inválido")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let dono = match texto("dono") {
            Some(d) => Cabeca::parse(&d).ok_or("dono inválido")?,
            None => cabeca,
        };
        let cmd = campos
            .get("cmd")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Guarda {
            cabeca,
            id: texto("id").ok_or("id ausente")?,
            tipo,
            momentos,
            titulo: texto("titulo").unwrap_or_default(),
            dono,
            cmd,
            timeout_s: campos.get("timeoutS").and_then(inteiro_de_timeout),
            campos,
        })
    }
}

impl From<Guarda> for Dict {
    fn from(g: Guarda) -> Dict {
        g.campos
    }
}

/// `Number.isInteger` sobre um número do JSON: `30` e `30.0` valem.
fn inteiro_de_timeout(v: &Value) -> Option<u64> {
    let n = v.as_f64()?;
    (n.fract() == 0.0 && (1.0..=TIMEOUT_MAX as f64).contains(&n)).then_some(n as u64)
}

/// `GUARD_ID` (`src/lib/cerberus.ts:28`): `^[a-z0-9][a-z0-9-]{0,47}$`.
fn id_valido(id: &str) -> bool {
    let b = id.as_bytes();
    let ok = |c: &u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    !b.is_empty() && b.len() <= 48 && ok(&b[0]) && b[1..].iter().all(|c| ok(c) || *c == b'-')
}

fn texto_cheio(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str).is_some_and(|s| !em_branco(s))
}

/// `guardErrors` (`src/lib/cerberus.ts:171`): os problemas do arquivo `<cab>/<id>.json`, na ordem do TS.
fn erros_da_guarda(registro: &Path, cab: Cabeca, id: &str, g: &Value) -> Vec<String> {
    let mut errs = Vec::new();
    if !id_valido(id) {
        errs.push("id inválido (nome do arquivo)".to_owned());
    }
    let Some(g) = g.as_object() else {
        errs.push("não é um objeto JSON".to_owned());
        return errs;
    };
    if !texto_cheio(g.get("titulo")) {
        errs.push("titulo ausente".to_owned());
    }
    if !texto_cheio(g.get("origem")) {
        errs.push("origem ausente".to_owned());
    }
    let momentos_ok = g
        .get("momentos")
        .and_then(Value::as_array)
        .is_some_and(|ms| {
            !ms.is_empty()
                && ms
                    .iter()
                    .all(|m| m.as_str().and_then(Momento::parse).is_some())
        });
    if !momentos_ok {
        errs.push(format!("momentos: lista não vazia de {}", Momento::lista()));
    }
    let tipo = g.get("tipo").and_then(Value::as_str).and_then(Tipo::parse);
    if tipo.is_none() {
        errs.push(format!("tipo: {}", Tipo::lista()));
    }
    if tipo == Some(Tipo::Exec) {
        erros_do_cmd(registro, cab, g.get("cmd"), &mut errs);
    }
    if g.contains_key("timeoutS") && g.get("timeoutS").and_then(inteiro_de_timeout).is_none() {
        errs.push(format!("timeoutS: inteiro de 1 a {TIMEOUT_MAX}"));
    }
    if tipo == Some(Tipo::Regra) {
        let dono_ok = match g.get("dono") {
            None | Some(Value::Null) => true,
            Some(d) => d.as_str().and_then(Cabeca::parse).is_some(),
        };
        if !dono_ok {
            errs.push("dono: porta, limiar ou vigilia".to_owned());
        }
    }
    errs
}

/// O trecho exec de `guardErrors` (`src/lib/cerberus.ts:182-194`).
fn erros_do_cmd(registro: &Path, cab: Cabeca, cmd: Option<&Value>, errs: &mut Vec<String>) {
    let partes: Option<Vec<&str>> = cmd.and_then(Value::as_array).and_then(|a| {
        a.iter()
            .map(|x| x.as_str().filter(|s| !s.is_empty()))
            .collect()
    });
    let Some(partes) = partes.filter(|p| !p.is_empty()) else {
        errs.push("cmd: lista de texto não vazia".to_owned());
        return;
    };
    let first = partes[0];
    if first == CLI_CMD {
        if partes.len() < 2 {
            errs.push(format!(
                "cmd: {CLI_CMD} leva o subcomando (ex.: {CLI_CMD} cerberus-check porta/<guarda>)"
            ));
        }
        return;
    }
    let base = realpath_missing_ok(&registro.join(cab.as_str()));
    if Path::new(first).is_absolute() {
        errs.push("cmd[0] tem de ser relativo à pasta da cabeça, sem sair dela".to_owned());
        return;
    }
    let target = realpath_missing_ok(&base.join(first));
    let dentro = target
        .to_string_lossy()
        .starts_with(&format!("{}/", base.to_string_lossy()));
    if !dentro {
        errs.push("cmd[0] tem de ser relativo à pasta da cabeça, sem sair dela".to_owned());
    } else if !is_file(&target) {
        errs.push(format!("cmd[0] não existe: {first}"));
    }
}

/// Guardas válidas e erros do registro. Arquivo inválido entra em erros, nunca some calado.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Registro {
    pub guardas: Vec<Guarda>,
    pub erros: Vec<String>,
}

impl Registro {
    /// `registry` (`src/lib/cerberus.ts:213`).
    pub fn carregar(skill: &Path) -> Registro {
        let registro = pasta(skill);
        let mut out = Registro::default();
        for &cab in Cabeca::TODOS {
            let dir = registro.join(cab.as_str());
            let nomes = if is_dir(&dir) {
                list_dir(&dir)
            } else {
                Vec::new()
            };
            for name in nomes {
                let Some(stem) = name.strip_suffix(".json") else {
                    continue;
                };
                let parsed = fs::read_to_string(dir.join(&name))
                    .map_err(|e| e.to_string())
                    .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
                let parsed = match parsed {
                    Ok(v) => v,
                    Err(e) => {
                        out.erros.push(format!("{cab}/{name}: {e}"));
                        continue;
                    }
                };
                let errs = erros_da_guarda(&registro, cab, stem, &parsed);
                if !errs.is_empty() {
                    out.erros.push(format!("{cab}/{name}: {}", errs.join("; ")));
                    continue;
                }
                match montar(cab, stem, parsed) {
                    Ok(g) => out.guardas.push(g),
                    Err(e) => out.erros.push(format!("{cab}/{name}: {e}")),
                }
            }
        }
        out
    }

    /// `findGuard` (`src/lib/cerberus.ts:240`).
    pub fn achar(&self, cab: &str, id: &str) -> Option<&Guarda> {
        self.guardas
            .iter()
            .find(|g| g.cabeca.as_str() == cab && g.id == id)
    }
}

/// `{ ...g, cabeca, id, dono }` (`src/lib/cerberus.ts:235`): chave que já existe fica no lugar dela.
fn montar(cab: Cabeca, id: &str, parsed: Value) -> Result<Guarda, String> {
    let Value::Object(mut g) = parsed else {
        return Err("não é um objeto JSON".to_owned());
    };
    let regra = g.get("tipo").and_then(Value::as_str) == Some(Tipo::Regra.as_str());
    let dono = match g.get("dono").and_then(Value::as_str) {
        Some(d) if regra => d.to_owned(),
        _ => cab.as_str().to_owned(),
    };
    g.insert("cabeca".into(), Value::from(cab.as_str()));
    g.insert("id".into(), Value::from(id));
    g.insert("dono".into(), Value::from(dono));
    Guarda::try_from(g)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testes::Caixa;

    const EMITIDA: &str =
        r#"{"titulo": "T", "origem": "o", "momentos": ["push"], "tipo": "emitida"}"#;

    #[test]
    fn carrega_valida_e_ordena() {
        let caixa = Caixa::nova();
        caixa.guarda("porta", "t-b", EMITIDA);
        caixa.guarda("porta", "t-a", r#"{"dono": "vigilia", "titulo": "A", "origem": "o", "momentos": ["done", "push"], "tipo": "regra", "timeoutS": 30.0}"#);
        caixa.guarda(
            "porta",
            "t-quebrada",
            r#"{"titulo": "T", "origem": "o", "momentos": ["push"], "tipo": "x"}"#,
        );
        caixa.escreve("skill/loop/cerberus/porta/LEIA.md", "não é guarda");
        caixa.guarda("limiar", "t-exec", r#"{"titulo": "T", "origem": "o", "momentos": ["push"], "tipo": "exec", "cmd": ["bin/check.sh", "x"]}"#);
        caixa.escreve("skill/loop/cerberus/limiar/bin/check.sh", "#!/bin/sh\n");
        caixa.guarda("vigilia", "t-cli", r#"{"titulo": "T", "origem": "o", "momentos": ["continuo"], "tipo": "exec", "cmd": ["hades", "cerberus-check"]}"#);

        let reg = Registro::carregar(caixa.raiz().join("skill").as_path());
        let ids: Vec<String> = reg
            .guardas
            .iter()
            .map(|g| format!("{}/{}", g.cabeca, g.id))
            .collect();
        assert_eq!(
            ids,
            ["porta/t-a", "porta/t-b", "limiar/t-exec", "vigilia/t-cli"]
        );
        assert_eq!(
            reg.erros,
            ["porta/t-quebrada.json: tipo: exec, emitida, regra"]
        );

        let a = reg.achar("porta", "t-a").unwrap();
        assert_eq!(a.dono, Cabeca::Vigilia);
        assert_eq!(a.timeout_s, Some(30));
        assert_eq!(
            a.campos.keys().collect::<Vec<_>>(),
            [
                "dono", "titulo", "origem", "momentos", "tipo", "timeoutS", "cabeca", "id"
            ]
        );
        let b = reg.achar("porta", "t-b").unwrap();
        assert_eq!(b.dono, Cabeca::Porta);
        assert_eq!(
            hades_core::jsonfmt::json_compact(&serde_json::to_value(b).unwrap()),
            r#"{"titulo":"T","origem":"o","momentos":["push"],"tipo":"emitida","cabeca":"porta","id":"t-b","dono":"porta"}"#
        );
        assert_eq!(
            reg.achar("limiar", "t-exec").unwrap().cmd,
            ["bin/check.sh", "x"]
        );
        assert!(reg.achar("porta", "nao").is_none());
    }

    #[test]
    fn erros_na_ordem_do_ts() {
        let caixa = Caixa::nova();
        caixa.guarda(
            "porta",
            "Ruim",
            r#"{"tipo": "exec", "cmd": ["../fora.sh"], "timeoutS": 0}"#,
        );
        caixa.guarda("porta", "lista", "[1]");
        caixa.guarda("porta", "quebrado", "{");
        caixa.guarda("limiar", "abs", r#"{"titulo": "T", "origem": "o", "momentos": ["push"], "tipo": "exec", "cmd": ["/bin/sh"]}"#);
        caixa.guarda("limiar", "falta", r#"{"titulo": "T", "origem": "o", "momentos": ["push"], "tipo": "exec", "cmd": ["nao.sh"]}"#);
        caixa.guarda("limiar", "vazio", r#"{"titulo": "T", "origem": "o", "momentos": ["push"], "tipo": "exec", "cmd": ["a", ""]}"#);
        caixa.guarda(
            "vigilia",
            "cli",
            r#"{"titulo": " ", "origem": "o", "momentos": [], "tipo": "exec", "cmd": ["hades"]}"#,
        );
        caixa.guarda(
            "vigilia",
            "dono",
            r#"{"titulo": "T", "origem": "o", "momentos": ["push"], "tipo": "regra", "dono": "x"}"#,
        );

        let reg = Registro::carregar(caixa.raiz().join("skill").as_path());
        assert!(reg.guardas.is_empty());
        let momentos = format!("momentos: lista não vazia de {}", Momento::lista());
        assert_eq!(
            reg.erros[0],
            format!(
                "porta/Ruim.json: id inválido (nome do arquivo); titulo ausente; origem ausente; {momentos}; \
             cmd[0] tem de ser relativo à pasta da cabeça, sem sair dela; timeoutS: inteiro de 1 a 600"
            )
        );
        assert_eq!(reg.erros[1], "porta/lista.json: não é um objeto JSON");
        assert!(reg.erros[2].starts_with("porta/quebrado.json: "));
        assert_eq!(
            reg.erros[3],
            "limiar/abs.json: cmd[0] tem de ser relativo à pasta da cabeça, sem sair dela"
        );
        assert_eq!(reg.erros[4], "limiar/falta.json: cmd[0] não existe: nao.sh");
        assert_eq!(
            reg.erros[5],
            "limiar/vazio.json: cmd: lista de texto não vazia"
        );
        assert_eq!(
            reg.erros[6],
            format!(
                "vigilia/cli.json: titulo ausente; {momentos}; cmd: hades leva o subcomando (ex.: hades cerberus-check porta/<guarda>)"
            )
        );
        assert_eq!(
            reg.erros[7],
            "vigilia/dono.json: dono: porta, limiar ou vigilia"
        );
    }

    #[test]
    fn sem_registro_nao_ha_guarda() {
        let caixa = Caixa::nova();
        assert_eq!(Registro::carregar(caixa.raiz()), Registro::default());
    }
}
