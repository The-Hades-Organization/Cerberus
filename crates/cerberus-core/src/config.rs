//! Chaves do catálogo e modo de cada cabeça (`src/lib/cerberus.ts:95-144`).
//!
//! Leitura própria, não o `config_value` do core: a precedência do Cerberus é variável exportada > `config.env` da
//! instalação > padrão do `config.tsv`, e o catálogo da skill vem antes do instalado (um `$D` de versão anterior ainda
//! não tem as chaves do Cerberus).

use std::path::PathBuf;

use hades_core::config::split_lines;
use hades_core::fsx::read_text;
use hades_core::jsonfmt::json_compact;
use hades_core::paths::hades_home;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::modelo::{Cabeca, Modo, js_num, js_space, js_trim, py_repr};
use crate::{Erro, Opcoes};

/// `Cfg` (`src/lib/cerberus.ts:45`): chave do catálogo para o valor efetivo, na ordem do catálogo.
pub type Cfg = IndexMap<String, String>;

/// `config` (`src/lib/cerberus.ts:97`). `com_env` falso lê só os arquivos (o painel, cujo ambiente é o da subida e pode
/// estar velho); verdadeiro, a variável exportada vence, vazia inclusive.
pub fn config(op: &Opcoes, com_env: bool) -> Cfg {
    let instancia = read_text(&hades_home(&op.env).join("instance")).unwrap_or_default();
    let primeira = js_trim(instancia.split('\n').next().unwrap_or(""));
    let data = (!primeira.is_empty()).then(|| PathBuf::from(primeira));

    let mut cfg = Cfg::new();
    let mut catalogos = vec![op.skill().join("kit").join("config.tsv")];
    if let Some(d) = &data {
        catalogos.push(d.join("config.tsv"));
    }
    for tsv in catalogos {
        let Some(text) = read_text(&tsv) else {
            continue;
        };
        for line in split_lines(&text) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut campos = line.split('\t');
            let key = campos.next().unwrap_or("");
            let value = campos.next().unwrap_or("");
            cfg.entry(key.to_owned())
                .or_insert_with(|| value.to_owned());
        }
    }

    if let Some(text) = data.and_then(|d| read_text(&d.join("config.env"))) {
        for line in split_lines(&text) {
            if let Some((key, value)) = line.split_once('=')
                && let Some(slot) = cfg.get_mut(key)
            {
                *slot = value.to_owned();
            }
        }
    }

    if com_env {
        for (key, slot) in cfg.iter_mut() {
            if let Some(value) = op.env.get_raw(key) {
                *slot = value.to_owned();
            }
        }
    }
    cfg
}

/// `/^\s*[+-]?\d+\s*$/` do JS.
fn inteiro_valido(raw: &str) -> bool {
    let corpo = raw.trim_matches(js_space);
    let digitos = corpo.strip_prefix(['+', '-']).unwrap_or(corpo);
    !digitos.is_empty() && digitos.bytes().all(|b| b.is_ascii_digit())
}

/// `cfgInt` (`src/lib/cerberus.ts:127`).
pub fn cfg_int(cfg: &Cfg, key: &str, lo: i64, hi: i64) -> Result<i64, Erro> {
    let raw = cfg
        .get(key)
        .filter(|raw| inteiro_valido(raw))
        .ok_or_else(|| {
            Erro::recusa(format!(
                "{key} ausente ou inválida no catálogo de configurações"
            ))
        })?;
    // `Number.parseInt` passa por `f64`: um texto longo demais sai arredondado e cai fora da faixa
    let value: f64 = raw.trim_matches(js_space).parse().unwrap_or(f64::NAN);
    if value < lo as f64 || value > hi as f64 {
        let texto = match js_num(value) {
            Value::Null => "NaN".to_owned(),
            n => json_compact(&n),
        };
        return Err(Erro::recusa(format!("{key}={texto} fora de {lo}–{hi}")));
    }
    Ok(value as i64)
}

/// Modo de cada cabeça (`src/lib/cerberus.ts:135`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Modos {
    pub porta: Modo,
    pub limiar: Modo,
    pub vigilia: Modo,
}

impl Modos {
    pub fn de(&self, cab: Cabeca) -> Modo {
        match cab {
            Cabeca::Porta => self.porta,
            Cabeca::Limiar => self.limiar,
            Cabeca::Vigilia => self.vigilia,
        }
    }
}

/// `modes` (`src/lib/cerberus.ts:135`).
pub fn modos(cfg: &Cfg) -> Result<Modos, Erro> {
    let um = |cab: Cabeca| -> Result<Modo, Erro> {
        let key = format!("HADES_CERBERUS_{}", cab.as_str().to_uppercase());
        let raw = cfg.get(&key).map(String::as_str).unwrap_or("");
        Modo::parse(raw).ok_or_else(|| {
            Erro::recusa(format!(
                "{key}={}: modo inválido (off, avisa ou barra)",
                py_repr(&Value::String(raw.to_owned()))
            ))
        })
    };
    Ok(Modos {
        porta: um(Cabeca::Porta)?,
        limiar: um(Cabeca::Limiar)?,
        vigilia: um(Cabeca::Vigilia)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testes::Caixa;

    fn cfg_de(pares: &[(&str, &str)]) -> Cfg {
        pares
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn precedencia_env_sobre_config_env_sobre_tsv() {
        let caixa = Caixa::nova();
        caixa.escreve(
            "skill/kit/config.tsv",
            "# comentário\n\nHADES_CERBERUS_PORTA\tbarra\tdescrição\nHADES_CERBERUS_LIMIAR\tbarra\nHADES_CERBERUS_TIMEOUT_S\t120\n",
        );
        let inst = caixa.raiz().join("INST");
        caixa.escreve(
            "home/.claude/hades/instance",
            &format!(" {} \nlixo\n", inst.display()),
        );
        caixa.escreve(
            "INST/config.tsv",
            "HADES_CERBERUS_PORTA\toff\nHADES_CERBERUS_VIGILIA\tavisa\n",
        );
        caixa.escreve(
            "INST/config.env",
            "HADES_CERBERUS_LIMIAR=avisa\nFORA_DO_CATALOGO=1\nsem-igual\n",
        );
        let op = caixa.opcoes().clone();
        let cfg = config(&op, false);
        assert_eq!(
            cfg.keys().collect::<Vec<_>>(),
            [
                "HADES_CERBERUS_PORTA",
                "HADES_CERBERUS_LIMIAR",
                "HADES_CERBERUS_TIMEOUT_S",
                "HADES_CERBERUS_VIGILIA"
            ]
        );
        assert_eq!(cfg["HADES_CERBERUS_PORTA"], "barra");
        assert_eq!(cfg["HADES_CERBERUS_LIMIAR"], "avisa");
        assert_eq!(cfg["HADES_CERBERUS_VIGILIA"], "avisa");

        let mut op = op;
        op.env = op
            .env
            .with("HADES_CERBERUS_TIMEOUT_S", "")
            .with("OUTRA", "x");
        assert_eq!(config(&op, false)["HADES_CERBERUS_TIMEOUT_S"], "120");
        let com_env = config(&op, true);
        assert_eq!(com_env["HADES_CERBERUS_TIMEOUT_S"], "");
        assert!(!com_env.contains_key("OUTRA"));
    }

    #[test]
    fn cfg_int_como_o_golden() {
        let cfg = cfg_de(&[("HADES_CERBERUS_TIMEOUT_S", " +3 "), ("X", "1.5")]);
        assert_eq!(
            cfg_int(&cfg, "HADES_CERBERUS_TIMEOUT_S", 1, 600).unwrap(),
            3
        );
        assert_eq!(
            cfg_int(&cfg, "HADES_CERBERUS_TIMEOUT_S", 10, 600)
                .unwrap_err()
                .to_string(),
            "HADES_CERBERUS_TIMEOUT_S=3 fora de 10–600"
        );
        assert_eq!(
            cfg_int(&cfg, "NAO_EXISTE", 1, 2).unwrap_err().to_string(),
            "NAO_EXISTE ausente ou inválida no catálogo de configurações"
        );
        assert!(cfg_int(&cfg, "X", 1, 2).is_err());
        let grande = cfg_de(&[("K", "99999999999999999999999")]);
        assert_eq!(
            cfg_int(&grande, "K", 1, 2).unwrap_err().to_string(),
            "K=1e+23 fora de 1–2"
        );
    }

    #[test]
    fn modos_como_o_golden() {
        let cfg = cfg_de(&[
            ("HADES_CERBERUS_PORTA", "barra"),
            ("HADES_CERBERUS_LIMIAR", "barra"),
            ("HADES_CERBERUS_VIGILIA", "avisa"),
        ]);
        let m = modos(&cfg).unwrap();
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"porta":"barra","limiar":"barra","vigilia":"avisa"}"#
        );
        assert_eq!(m.de(Cabeca::Vigilia), Modo::Avisa);
        let mut ruim = cfg.clone();
        ruim.insert("HADES_CERBERUS_PORTA".into(), "x'y".into());
        assert_eq!(
            modos(&ruim).unwrap_err().to_string(),
            "HADES_CERBERUS_PORTA=\"x'y\": modo inválido (off, avisa ou barra)"
        );
        assert_eq!(
            modos(&Cfg::new()).unwrap_err().to_string(),
            "HADES_CERBERUS_PORTA='': modo inválido (off, avisa ou barra)"
        );
    }
}
