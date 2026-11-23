//! Caixa de areia dos testes: `HOME`, skill e raiz de trabalho numa pasta temporária, nunca os reais.

use std::fs;
use std::path::{Path, PathBuf};

use hades_core::Env;

use crate::Opcoes;

pub struct Caixa {
    _dir: tempfile::TempDir,
    raiz: PathBuf,
    opcoes: Opcoes,
}

impl Caixa {
    pub fn nova() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let raiz = fs::canonicalize(dir.path()).unwrap();
        let env = Env::empty()
            .with("HOME", raiz.join("home").to_string_lossy())
            .with("HADES_WORK_ROOT", raiz.join("work").to_string_lossy());
        let opcoes = Opcoes::novo(env, raiz.clone()).com_skill(raiz.join("skill"));
        Self {
            _dir: dir,
            raiz,
            opcoes,
        }
    }

    pub fn raiz(&self) -> &Path {
        &self.raiz
    }

    pub fn opcoes(&self) -> &Opcoes {
        &self.opcoes
    }

    pub fn escreve(&self, rel: &str, texto: &str) -> PathBuf {
        let path = self.raiz.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, texto).unwrap();
        path
    }

    /// Guarda no registro da skill: `loop/cerberus/<cab>/<id>.json`.
    pub fn guarda(&self, cab: &str, id: &str, json: &str) -> PathBuf {
        self.escreve(&format!("skill/loop/cerberus/{cab}/{id}.json"), json)
    }
}
