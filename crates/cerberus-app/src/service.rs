//! Leitura que o app faz sobre o `cerberus-core`; nada aqui roda na thread da interface.

use std::path::{Path, PathBuf};

use cerberus_core::config::cfg_int;
use cerberus_core::modelo::{Ctx, Dict, Excecao, Veredito, now_secs};
use cerberus_core::registro::Guarda;
use cerberus_core::{Cerberus, Opcoes, PedidoExcecao};
use hades_core::Env;
use hades_core::fsx::read_text;
use hades_core::paths::hades_home;

use crate::vm::{CABECAS, CartaoModo, chave_do_modo, origem_do_modo};

/// Tudo o que as telas leem, numa ida ao disco.
#[derive(Clone, Debug)]
pub struct Dados {
    pub historico: Vec<Dict>,
    pub guardas: Vec<Guarda>,
    pub erros: Vec<String>,
    /// Modo efetivo por cabeça, ou a recusa do core quando um valor é inválido.
    pub modos: Result<Vec<CartaoModo>, String>,
    pub agora_s: f64,
    pub excecoes: Result<Vec<Excecao>, String>,
    /// `HADES_CERBERUS_EXC_TTL_H` efetivo: teto e padrão da validade de uma exceção nova.
    pub ttl_h: Result<i64, String>,
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
        excecoes: cerberus.excecoes().map_err(|e| e.to_string()),
        ttl_h: cfg_int(&cerberus.config(true), "HADES_CERBERUS_EXC_TTL_H", 1, 720)
            .map_err(|e| e.to_string()),
        pasta,
    })
}

/// Exceção pedida pelo formulário da tela Exceções.
#[derive(Clone, Debug)]
pub struct NovaExcecao {
    pub cabeca: String,
    pub guarda: String,
    pub momento: String,
    pub motivo: String,
    pub alvo: Vec<String>,
    pub ttl_h: i64,
}

/// Abre a exceção pela `abrir_excecao` do core, com as mesmas validações do CLI; a recusa volta como texto.
pub fn abrir_excecao(opcoes: Opcoes, n: NovaExcecao) -> Result<Excecao, String> {
    let mut p = PedidoExcecao::default();
    p.cabeca = n.cabeca;
    p.guarda = n.guarda;
    p.momento = n.momento;
    p.motivo = n.motivo;
    p.alvo = n.alvo;
    p.origem = "app".into();
    p.ttl_s = Some(n.ttl_h * 3600);
    Cerberus::abrir(opcoes)
        .abrir_excecao(p)
        .map_err(|e| e.to_string())
}

/// Roda a guarda exec sobre o texto ou o arquivo dado, sem gravar veredito.
pub fn ensaiar(
    opcoes: Opcoes,
    cabeca: &str,
    guarda: &str,
    momento: &str,
    ctx: Ctx,
) -> Result<Veredito, String> {
    Cerberus::abrir(opcoes)
        .ensaiar(cabeca, guarda, momento, &ctx, None)
        .map_err(|e| e.to_string())
}

/// Opções do core para o app: o ambiente do processo e a pasta onde ele abriu.
pub fn opcoes(env: Env, cwd: &Path) -> Opcoes {
    Opcoes::novo(env, cwd.to_path_buf())
}

#[cfg(test)]
mod tests;
