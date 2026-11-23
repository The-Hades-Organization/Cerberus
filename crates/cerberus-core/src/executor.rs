//! Execução das guardas `exec` (`src/lib/cerberus.ts:316-376`): o subprocesso atrás de uma trait, para o app e os
//! testes trocarem o executor, e a leitura do veredito que a guarda imprime.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hades_core::config::split_lines;
use hades_core::fsx::realpath_missing_ok;
use hades_core::jsonfmt::stringify;
use hades_core::proc::{RunOptions, run, which};
use serde_json::{Map, Value};

use crate::ctx::{erro_do_veredito, recusado};
use crate::modelo::{Ctx, OUT_BUFFER, OUT_MAX, Veredito, js_norm, js_trim};
use crate::registro::{CLI_CMD, Guarda, pasta};

/// O que roda: o binário e os argumentos, a pasta, as variáveis somadas às do processo, o stdin e o tempo limite.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Pedido {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub stdin: Vec<u8>,
    pub timeout: Duration,
}

/// Como o subprocesso acabou.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Saida {
    /// Saiu com o código dado; `stdout` é a saída padrão inteira.
    Terminou { status: i32, stdout: String },
    /// Passou do tempo limite e foi morto.
    Tempo,
    /// Nem chegou a rodar (ou estourou o buffer); o texto é o do `errno`, como o `ERRNO_TEXT` do TS.
    NaoExecutou(String),
}

/// Quem roda as guardas.
pub trait Executor: Send + Sync {
    fn executar(&self, pedido: &Pedido) -> Saida;
}

/// O executor de verdade: [`hades_core::proc::run`], com o filho no próprio grupo de processos, morto inteiro no
/// tempo limite. O filho herda o ambiente do processo mais [`Pedido::env`] (o `Env` do core não itera as variáveis).
#[derive(Clone, Copy, Debug, Default)]
pub struct ExecutorProcesso;

const ENOENT: &str = "No such file or directory";
const EACCES: &str = "Permission denied";
const ENOTDIR: &str = "Not a directory";

/// O `errno` que o `spawnSync` do Node daria antes de rodar (`ERRNO_TEXT`, `src/lib/cerberus.ts:318`), já que o
/// `proc::run` do core devolve só status 1 quando o spawn falha.
fn erro_de_spawn(argv0: &str, cwd: &Path) -> Option<&'static str> {
    match fs::metadata(cwd) {
        Err(_) => return Some(ENOENT),
        Ok(m) if !m.is_dir() => return Some(ENOTDIR),
        Ok(_) => {}
    }
    let alvo = if argv0.contains('/') {
        PathBuf::from(argv0)
    } else {
        let path = std::env::var("PATH").unwrap_or_default();
        return which(argv0, &path).is_none().then_some(ENOENT);
    };
    match fs::metadata(&alvo) {
        Err(_) => Some(ENOENT),
        Ok(m) if m.is_dir() || m.permissions().mode() & 0o111 == 0 => Some(EACCES),
        Ok(_) => None,
    }
}

impl Executor for ExecutorProcesso {
    /// `spawnSync` de `runGuard` (`src/lib/cerberus.ts:357`).
    fn executar(&self, pedido: &Pedido) -> Saida {
        let Some((argv0, args)) = pedido.argv.split_first() else {
            return Saida::NaoExecutou(ENOENT.to_owned());
        };
        if let Some(texto) = erro_de_spawn(argv0, &pedido.cwd) {
            return Saida::NaoExecutou(texto.to_owned());
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let r = run(
            argv0,
            &args,
            &RunOptions {
                cwd: Some(pedido.cwd.clone()),
                timeout: Some(pedido.timeout),
                max_buffer: Some(OUT_BUFFER),
                env: pedido.env.clone(),
                stdin: Some(pedido.stdin.clone()),
            },
        );
        if r.timed_out {
            return Saida::Tempo;
        }
        if r.stdout.len() >= OUT_BUFFER || r.stderr.len() >= OUT_BUFFER {
            return Saida::NaoExecutou(format!("spawnSync {argv0} ENOBUFS"));
        }
        Saida::Terminou {
            status: r.status,
            stdout: r.stdout,
        }
    }
}

/// `slice(0, n)` do JS: as primeiras `n` unidades UTF-16. O par substituto que ficaria partido sai inteiro.
fn corta_utf16(text: &str, n: usize) -> &str {
    let mut unidades = 0;
    for (i, c) in text.char_indices() {
        unidades += c.len_utf16();
        if unidades > n {
            return &text[..i];
        }
    }
    text
}

/// `lastJsonLine` (`src/lib/cerberus.ts:333`): a última linha como JSON; `None` se não há linha, não é JSON ou é
/// `null`.
fn ultima_linha_json(out: &str) -> Option<Value> {
    let last = split_lines(out).pop()?;
    match serde_json::from_str::<Value>(&last) {
        Ok(Value::Null) | Err(_) => None,
        Ok(v) => Some(js_norm(v)),
    }
}

/// Onde a guarda roda e qual binário atende `cmd[0] == "hades"`.
pub(crate) struct Lugar<'a> {
    pub skill: &'a Path,
    pub cwd: &'a Path,
    pub bin_check: Option<&'a Path>,
}

/// `argv` e `where` de `runGuard` (`src/lib/cerberus.ts:346-348`). Com `bin_check`, a guarda
/// `hades cerberus-check …` roda `<bin_check> check …` (divergência D2: o próprio binário, não o `bun` da skill).
fn argv_e_onde(g: &Guarda, lugar: &Lugar) -> (Vec<String>, String) {
    let cmd = &g.cmd;
    let cmd0 = cmd.first().map(String::as_str).unwrap_or("");
    if cmd0 == CLI_CMD {
        let onde = cmd.join(" ");
        let argv = match lugar.bin_check {
            Some(bin) if cmd.get(1).map(String::as_str) == Some("cerberus-check") => {
                let mut argv = vec![bin.to_string_lossy().into_owned(), "check".to_owned()];
                argv.extend(cmd[2..].iter().cloned());
                argv
            }
            _ => cmd.clone(),
        };
        return (argv, onde);
    }
    let cab = g.cabeca.as_str();
    let bin = realpath_missing_ok(&pasta(lugar.skill).join(cab).join(cmd0));
    let mut argv = vec![bin.to_string_lossy().into_owned()];
    argv.extend(cmd.iter().skip(1).cloned());
    (argv, format!("loop/cerberus/{cab}/{cmd0}"))
}

/// `runGuard` (`src/lib/cerberus.ts:344`): roda a guarda exec e devolve o veredito dela, ou o barra de recusa se ele
/// não vale.
pub(crate) fn rodar_guarda(
    ex: &dyn Executor,
    g: &Guarda,
    momento: &str,
    ctx: &Ctx,
    lugar: &Lugar,
    timeout_padrao: u64,
) -> Veredito {
    let cab = g.cabeca.as_str();
    let id = g.id.as_str();
    let (argv, onde) = argv_e_onde(g, lugar);
    let timeout_s = g.timeout_s.unwrap_or(timeout_padrao);
    let ctx_obj: Map<String, Value> = ctx
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    let payload = stringify(&Value::Object(ctx_obj));
    let mut env = vec![
        ("CERBERUS_CABECA".to_owned(), cab.to_owned()),
        ("CERBERUS_GUARDA".to_owned(), id.to_owned()),
        ("CERBERUS_MOMENTO".to_owned(), momento.to_owned()),
        (
            "CERBERUS_DIR".to_owned(),
            lugar.cwd.to_string_lossy().into_owned(),
        ),
        (
            "CERBERUS_SKILL".to_owned(),
            lugar.skill.to_string_lossy().into_owned(),
        ),
        ("CERBERUS_CTX".to_owned(), payload.clone()),
    ];
    for (k, v) in ctx {
        env.push((format!("CERBERUS_CTX_{}", k.to_uppercase()), v.clone()));
    }
    let pedido = Pedido {
        argv,
        cwd: lugar.cwd.to_path_buf(),
        env,
        stdin: payload.into_bytes(),
        timeout: Duration::from_secs(timeout_s),
    };
    let (status, stdout) = match ex.executar(&pedido) {
        Saida::Tempo => {
            return recusado(cab, id, momento, &format!("passou de {timeout_s} s"), &onde);
        }
        Saida::NaoExecutou(texto) => {
            return recusado(cab, id, momento, &format!("não executou: {texto}"), &onde);
        }
        Saida::Terminou { status, stdout } => (status, stdout),
    };
    let out = js_trim(corta_utf16(&stdout, OUT_MAX));
    let v = if out.is_empty() {
        None
    } else {
        ultima_linha_json(out)
    };
    let why = match &v {
        Some(v) => erro_do_veredito(v, cab, id, momento),
        None if status != 0 => Some(format!("saiu {status} sem veredito")),
        None => Some("saída sem veredito JSON".to_owned()),
    };
    if let Some(why) = why {
        return recusado(cab, id, momento, &why, &onde);
    }
    let Some(Value::Object(verdict)) = v else {
        unreachable!("erro_do_veredito recusa o que não é objeto")
    };
    let mut picked = Map::new();
    for key in ["resultado", "evidencia", "detalhe"] {
        if let Some(value) = verdict.get(key) {
            picked.insert(key.to_owned(), value.clone());
        }
    }
    picked.insert("cabeca".into(), cab.into());
    picked.insert("guarda".into(), id.into());
    picked.insert("momento".into(), momento.into());
    Veredito(picked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corta_em_unidades_utf16() {
        assert_eq!(corta_utf16("abc", 2), "ab");
        assert_eq!(corta_utf16("a😀b", 2), "a");
        assert_eq!(corta_utf16("a😀b", 3), "a😀");
        assert_eq!(corta_utf16("ç", 5), "ç");
    }

    #[test]
    fn ultima_linha_como_o_ts() {
        assert_eq!(
            ultima_linha_json("lixo\n{\"a\": 1}"),
            Some(serde_json::json!({"a": 1}))
        );
        assert_eq!(ultima_linha_json("{\"a\": 1}\nlixo"), None);
        assert_eq!(ultima_linha_json("null"), None);
        assert_eq!(ultima_linha_json(""), None);
    }

    #[test]
    fn spawn_recusado_antes_de_rodar() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path();
        assert_eq!(erro_de_spawn("/nao/existe", cwd), Some(ENOENT));
        assert_eq!(erro_de_spawn("/bin/sh", &cwd.join("x")), Some(ENOENT));
        let arq = cwd.join("f");
        fs::write(&arq, "x").unwrap();
        assert_eq!(erro_de_spawn("/bin/sh", &arq), Some(ENOTDIR));
        assert_eq!(erro_de_spawn(&arq.to_string_lossy(), cwd), Some(EACCES));
        assert_eq!(erro_de_spawn(&cwd.to_string_lossy(), cwd), Some(EACCES));
        assert_eq!(
            erro_de_spawn("comando-que-nao-existe-xyz", cwd),
            Some(ENOENT)
        );
        assert_eq!(erro_de_spawn("/bin/sh", cwd), None);
    }
}
