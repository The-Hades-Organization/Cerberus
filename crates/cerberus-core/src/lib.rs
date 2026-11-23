//! Regra de negócio do Cerberus, sem interface: o corpo comum das três cabeças (`src/lib/cerberus.ts` da skill).
//!
//! Registro de guardas lido de `<skill>/loop/cerberus` (nunca embutido no binário), modo por cabeça, veredito com
//! evidência, exceção do dono e histórico fora do repositório. Os arquivos de estado (`vereditos.jsonl`,
//! `excecoes.json`) saem byte a byte iguais aos do TS.

pub mod config;
pub mod ctx;
pub mod efeito;
pub mod modelo;
pub mod registro;
pub mod store;

#[cfg(test)]
mod testes;

use std::fmt;
use std::io;
use std::path::PathBuf;

use hades_core::Env;

/// O que toda operação do Cerberus recebe: ambiente, pasta de trabalho e, opcionalmente, a skill e o binário que
/// atende `cmd[0] == "hades"`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Opcoes {
    pub env: Env,
    pub cwd: PathBuf,
    /// Raiz da skill; ausente, vale [`hades_core::paths::skill_root`].
    pub skill: Option<PathBuf>,
    /// Binário que roda as guardas `hades <subcomando>`; ausente, o executor procura `hades` no `PATH`.
    pub bin_check: Option<PathBuf>,
}

impl Opcoes {
    pub fn novo(env: Env, cwd: PathBuf) -> Self {
        Self {
            env,
            cwd,
            skill: None,
            bin_check: None,
        }
    }

    pub fn com_skill(mut self, skill: PathBuf) -> Self {
        self.skill = Some(skill);
        self
    }

    pub fn com_bin_check(mut self, bin: PathBuf) -> Self {
        self.bin_check = Some(bin);
        self
    }

    /// `skillRoot` (`src/lib/paths.ts:4`): a dada ou a instalada.
    pub fn skill(&self) -> PathBuf {
        self.skill
            .clone()
            .unwrap_or_else(|| hades_core::paths::skill_root(&self.env))
    }
}

/// `Recusa` (`src/lib/cerberus.ts:76`): entrada inválida, a CLI sai 2 com a mensagem. `Io` é falha de disco.
#[derive(Debug)]
#[non_exhaustive]
pub enum Erro {
    Recusa(String),
    Io(io::Error),
}

impl Erro {
    pub(crate) fn recusa(texto: impl Into<String>) -> Self {
        Erro::Recusa(texto.into())
    }
}

impl fmt::Display for Erro {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Erro::Recusa(texto) => f.write_str(texto),
            Erro::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Erro {}

impl From<io::Error> for Erro {
    fn from(e: io::Error) -> Self {
        Erro::Io(e)
    }
}
