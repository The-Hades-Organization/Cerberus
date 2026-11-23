//! Leitura que o app faz sobre o `cerberus-core`; nada aqui roda na thread da interface.

use std::path::{Path, PathBuf};

use cerberus_core::modelo::{Dict, now_secs};
use cerberus_core::registro::Guarda;
use cerberus_core::{Cerberus, Opcoes};
use hades_core::Env;
use hades_core::fsx::read_text;
use hades_core::paths::hades_home;

use crate::vm::{CABECAS, CartaoModo, chave_do_modo, origem_do_modo};

/// Tudo o que as telas Vereditos, Guardas e Modos leem, numa ida ao disco.
#[derive(Clone, Debug)]
pub struct Dados {
    pub historico: Vec<Dict>,
    pub guardas: Vec<Guarda>,
    pub erros: Vec<String>,
    /// Modo efetivo por cabeça, ou a recusa do core quando um valor é inválido.
    pub modos: Result<Vec<CartaoModo>, String>,
    pub agora_s: f64,
    /// Pasta do histórico (`vereditos.jsonl`), para o rodapé.
    pub pasta: Option<PathBuf>,
}

/// `$D` da instalação: primeira linha de `~/.hades/instance`, como o `config` do core lê.
fn pasta_da_instalacao(env: &Env) -> Option<PathBuf> {
    let texto = read_text(&hades_home(env).join("instance"))?;
    let primeira = texto.lines().next()?.trim();
    (!primeira.is_empty()).then(|| PathBuf::from(primeira))
}

fn cartoes_de_modo(cerberus: &Cerberus, env: &Env) -> Result<Vec<CartaoModo>, String> {
    let modos = cerberus.modos().map_err(|e| e.to_string())?;
    let config_env = pasta_da_instalacao(env).and_then(|d| read_text(&d.join("config.env")));
    Ok(CABECAS
        .iter()
        .map(|cab| {
            let chave = chave_do_modo(*cab);
            let origem =
                origem_do_modo(&chave, env.get_raw(&chave).is_some(), config_env.as_deref());
            CartaoModo {
                cabeca: *cab,
                chave,
                modo: modos.de(*cab),
                origem,
            }
        })
        .collect())
}

pub fn carregar(opcoes: Opcoes) -> Result<Dados, String> {
    let env = opcoes.env.clone();
    let cerberus = Cerberus::abrir(opcoes);
    let historico = cerberus
        .historico(None, None, None)
        .map_err(|e| e.to_string())?;
    let (guardas, erros) = cerberus.guardas(None);
    let pasta = cerberus_core::store::Store::de(cerberus.opcoes())
        .ok()
        .map(|s| s.dir().to_path_buf());
    Ok(Dados {
        historico,
        guardas,
        erros,
        modos: cartoes_de_modo(&cerberus, &env),
        agora_s: now_secs(),
        pasta,
    })
}

/// Opções do core para o app: o ambiente do processo e a pasta onde ele abriu.
pub fn opcoes(env: Env, cwd: &Path) -> Opcoes {
    Opcoes::novo(env, cwd.to_path_buf())
}

#[cfg(test)]
mod tests;
