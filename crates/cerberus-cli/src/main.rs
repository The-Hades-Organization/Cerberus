//! `cerberus`: a CLI do Cerberus (`src/commands/cerberus.ts` da skill), com os mesmos textos, códigos de saída e
//! ordem de chaves do `--json`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cerberus_core::check::CheckEnv;
use cerberus_core::modelo::{Dict, js_string, now_iso, py_repr};
use cerberus_core::{
    Cerberus, Entrada, Erro, Opcoes, PedidoExcecao, PedidoRegistro, ctx::parse_ctx,
};
use hades_core::Env;
use hades_core::jsonfmt::py_dumps;
use serde_json::{Value, json};

/// `USAGE` (`src/commands/cerberus.ts:9`).
const USAGE: &str = "  avaliar <momento> [--dir <d>] [--ctx k=v]... [--cabeca <c>] [--json]   exit 0 passa, 1 barra, 2 uso/registro inválido
  registrar --cabeca <c> --guarda <id> --momento <m> --resultado <r> --evidencia \"<conferido>|<onde>\"... [--detalhe <t>]
            [--efeito <e>] [--ctx k=v]... [--dir <d>]                     veredito de guarda `emitida`; exit 2 se recusado
  guardas [--cabeca <c>] [--validar] [--json]                             o registro (--validar: exit 2 com os erros)
  historico [--cabeca <c>] [--efeito <e>] [--limite N] [--json]
  modos [--json]
  excecao listar [--json]
  excecao abrir --cabeca <c> --guarda <id> --momento <m> --motivo \"<t>\" [--alvo k=v]...   só pelo `!` do dono
  estado --json                                                          o que a página do painel mostra";

/// `EXIT` (`src/lib/output.ts:2`).
const OK: i32 = 0;
const FALHA: i32 = 1;
const USO: i32 = 2;

/// `HISTORY_DEFAULT` (`src/commands/cerberus.ts:19`).
const HISTORICO_PADRAO: usize = 50;
const FLAGS: [&str; 2] = ["--json", "--validar"];
const MULTI: [&str; 3] = ["--ctx", "--evidencia", "--alvo"];

/// `Parsed` (`src/commands/cerberus.ts:23`).
#[derive(Default)]
struct Args {
    pos: Vec<String>,
    opt: Vec<(String, Option<String>)>,
    ctx: Vec<String>,
    evidencia: Vec<String>,
    alvo: Vec<String>,
}

impl Args {
    /// `parseArgs` (`src/commands/cerberus.ts:29`).
    fn parse(argv: &[String]) -> Result<Self, Erro> {
        let mut a = Args::default();
        let mut i = 0;
        while i < argv.len() {
            let arg = &argv[i];
            if FLAGS.contains(&arg.as_str()) {
                a.definir(arg, None);
            } else if arg.starts_with("--") {
                let Some(valor) = argv.get(i + 1) else {
                    return Err(recusa(format!("{arg} pede um valor")));
                };
                match MULTI.iter().position(|k| k == arg) {
                    Some(0) => a.ctx.push(valor.clone()),
                    Some(1) => a.evidencia.push(valor.clone()),
                    Some(_) => a.alvo.push(valor.clone()),
                    None => a.definir(arg, Some(valor.clone())),
                }
                i += 1;
            } else {
                a.pos.push(arg.clone());
            }
            i += 1;
        }
        Ok(a)
    }

    fn definir(&mut self, chave: &str, valor: Option<String>) {
        match self.opt.iter_mut().find(|(k, _)| k == chave) {
            Some(par) => par.1 = valor,
            None => self.opt.push((chave.to_owned(), valor)),
        }
    }

    /// `str` (`src/commands/cerberus.ts:48`): só valor de texto; flag conta como ausente.
    fn texto(&self, chave: &str) -> Option<&str> {
        self.opt
            .iter()
            .find(|(k, _)| k == chave)
            .and_then(|(_, v)| v.as_deref())
    }

    fn flag(&self, chave: &str) -> bool {
        self.opt.iter().any(|(k, v)| k == chave && v.is_none())
    }

    /// `need` (`src/commands/cerberus.ts:75`): texto vazio também falta.
    fn exigir(&self, what: &str, chaves: &[&str]) -> Result<(), Erro> {
        for chave in chaves {
            if self.texto(chave).is_none_or(str::is_empty) {
                return Err(recusa(format!("{what} pede {chave}")));
            }
        }
        Ok(())
    }

    fn obrigatorio(&self, chave: &str) -> String {
        self.texto(chave).unwrap_or_default().to_owned()
    }
}

fn recusa(texto: String) -> Erro {
    Erro::Recusa(texto)
}

/// `padEnd` do JS: completa com espaços até `n` unidades UTF-16.
fn pad(texto: &str, n: usize) -> String {
    let len = texto.encode_utf16().count();
    format!("{texto}{}", " ".repeat(n.saturating_sub(len)))
}

/// Verdade do JS para um valor JSON.
fn verdade(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `a || b` do JS sobre valores JSON.
fn ou<'a>(a: Option<&'a Value>, b: Option<&'a Value>) -> Option<&'a Value> {
    if verdade(a) { a } else { b }
}

fn primeira_evidencia(r: &Dict) -> Option<&Dict> {
    r.get("evidencia")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_object)
}

/// `evidenceArgs` (`src/commands/cerberus.ts:53`).
fn evidencias(itens: &[String]) -> Result<Value, Erro> {
    let mut lista = Vec::new();
    for item in itens {
        let Some(bar) = item.rfind('|') else {
            return Err(recusa(format!(
                "--evidencia {}: \"<conferido>|<onde>\"",
                py_repr(&Value::String(item.clone()))
            )));
        };
        lista.push(json!({
            "conferido": cerberus_core::modelo::js_trim(&item[..bar]),
            "onde": cerberus_core::modelo::js_trim(&item[bar + 1..]),
        }));
    }
    Ok(Value::Array(lista))
}

/// `MARK` (`src/commands/cerberus.ts:61`).
fn marca(efeito: &str) -> &'static str {
    match efeito {
        "ok" => "✓",
        "aviso" => "!",
        "liberado" => "~",
        "barra" => "✗",
        _ => "undefined",
    }
}

/// Saída da CLI: `out` (stdout) e `warn` (stderr), linha a linha.
struct Saida<'a> {
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
}

impl Saida<'_> {
    fn out(&mut self, texto: &str) {
        let _ = writeln!(self.out, "{texto}");
    }

    fn warn(&mut self, texto: &str) {
        let _ = writeln!(self.err, "{texto}");
    }
}

fn dumps<T: serde::Serialize>(valor: &T) -> String {
    py_dumps(&serde_json::to_value(valor).unwrap_or(Value::Null), None)
}

/// `printEval` (`src/commands/cerberus.ts:63`).
fn imprimir_avaliacao(s: &mut Saida, r: &cerberus_core::Avaliacao) {
    if r.vereditos.is_empty() {
        return;
    }
    for v in &r.vereditos {
        let note = match v.get("excecao").and_then(Value::as_object) {
            Some(exc) => format!(
                " [exceção {}: {}]",
                js_string(exc.get("id")),
                js_string(exc.get("motivo"))
            ),
            None => String::new(),
        };
        let first = primeira_evidencia(v).and_then(|e| e.get("conferido"));
        let efeito = js_string(v.get("efeito"));
        s.out(&format!(
            "  {} {} {}/{}: {}{note}",
            marca(&efeito),
            pad(&efeito, 8),
            js_string(v.get("cabeca")),
            js_string(v.get("guarda")),
            js_string(ou(v.get("detalhe"), first)),
        ));
    }
    let off = if r.ignoradas.is_empty() {
        String::new()
    } else {
        format!(" (cabeça em off: {})", r.ignoradas.join(", "))
    };
    let resultado = serde_json::to_value(r.resultado).unwrap_or(Value::Null);
    s.out(&format!(
        "cerberus {}: {} — {} veredito(s), {} barra(m), {} aviso(s){off}",
        r.momento,
        js_string(Some(&resultado)),
        r.vereditos.len(),
        r.barras,
        r.avisos
    ));
}

fn segundos_agora() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// `run` (`src/commands/cerberus.ts:79`).
fn rodar(argv: &[String], op: &Opcoes, s: &mut Saida) -> Result<i32, Erro> {
    let cmd = argv.first().map(String::as_str).unwrap_or("");
    let a = Args::parse(argv.get(1..).unwrap_or(&[]))?;
    let json = a.flag("--json");
    let cer = Cerberus::abrir(op.clone());
    match cmd {
        "avaliar" => {
            if a.pos.len() != 1 {
                return Err(recusa("avaliar <momento>".into()));
            }
            let ctx = parse_ctx(&a.ctx)?;
            let dir = a.texto("--dir").filter(|d| !d.is_empty()).map(Path::new);
            let dir = hades_core::fsx::resolve(&op.cwd.join(dir.unwrap_or(&op.cwd)));
            let mut entrada = Entrada::default();
            entrada.ctx = ctx;
            entrada.cwd = Some(dir.to_string_lossy().into_owned());
            entrada.cabeca = a.texto("--cabeca").map(str::to_owned);
            let r = cer.avaliar(&a.pos[0], entrada)?;
            if json {
                s.out(&dumps(&r));
            } else {
                imprimir_avaliacao(s, &r);
            }
            Ok(if r.barrou() { FALHA } else { OK })
        }
        "registrar" => {
            a.exigir("registrar", &["--cabeca", "--guarda", "--momento", "--resultado"])?;
            let mut p = PedidoRegistro::default();
            p.evidencia = evidencias(&a.evidencia)?;
            p.ctx = parse_ctx(&a.ctx)?;
            p.cabeca = a.obrigatorio("--cabeca");
            p.guarda = a.obrigatorio("--guarda");
            p.momento = a.obrigatorio("--momento");
            p.resultado = a.obrigatorio("--resultado");
            p.detalhe = a.texto("--detalhe").map(str::to_owned);
            p.efeito = a.texto("--efeito").map(str::to_owned);
            p.cwd = a.texto("--dir").map(str::to_owned);
            cer.registrar(p)?;
            Ok(OK)
        }
        "guardas" => {
            let (guardas, erros) = cer.guardas(a.texto("--cabeca"));
            if json {
                s.out(&dumps(&json!({ "guardas": guardas, "erros": erros })));
            } else {
                for g in &guardas {
                    let momentos: Vec<&str> = g.momentos.iter().map(|m| m.as_str()).collect();
                    s.out(&format!(
                        "{} {} {} {} {}",
                        pad(g.cabeca.as_str(), 8),
                        pad(&g.id, 28),
                        pad(g.tipo.as_str(), 8),
                        pad(&momentos.join(","), 20),
                        g.titulo
                    ));
                }
                for e in &erros {
                    s.warn(&format!("✗ {e}"));
                }
            }
            Ok(if !erros.is_empty() && a.flag("--validar") { USO } else { OK })
        }
        "historico" => {
            let limite = a.texto("--limite");
            let limite = match limite {
                None => HISTORICO_PADRAO,
                Some(l) if !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()) && !l.trim_start_matches('0').is_empty() => {
                    l.parse().unwrap_or(usize::MAX)
                }
                Some(_) => return Err(recusa("--limite: inteiro positivo".into())),
            };
            let rows = cer.historico(a.texto("--cabeca"), a.texto("--efeito"), Some(limite))?;
            if json {
                s.out(&dumps(&rows));
            } else {
                for r in &rows {
                    let first = primeira_evidencia(r).and_then(|e| e.get("conferido"));
                    let vazio = Value::String(String::new());
                    s.out(&format!(
                        "{} {} {}/{} @{}: {}",
                        js_string(r.get("ts")),
                        pad(&js_string(r.get("efeito")), 8),
                        js_string(r.get("cabeca")),
                        js_string(r.get("guarda")),
                        js_string(r.get("momento")),
                        js_string(ou(ou(r.get("detalhe"), first), Some(&vazio))),
                    ));
                }
            }
            Ok(OK)
        }
        "modos" => {
            let m = cer.modos()?;
            if json {
                s.out(&dumps(&m));
            } else {
                let v = serde_json::to_value(m).unwrap_or(Value::Null);
                let linhas: Vec<String> = v
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(c, v)| format!("{} {}", pad(c, 8), js_string(Some(v))))
                    .collect();
                s.out(&linhas.join("\n"));
            }
            Ok(OK)
        }
        "excecao" => match a.pos.first().map(String::as_str) {
            Some("abrir") => {
                a.exigir("excecao abrir", &["--cabeca", "--guarda", "--momento", "--motivo"])?;
                let mut p = PedidoExcecao::default();
                p.cabeca = a.obrigatorio("--cabeca");
                p.guarda = a.obrigatorio("--guarda");
                p.momento = a.obrigatorio("--momento");
                p.motivo = a.obrigatorio("--motivo");
                p.alvo = a.alvo.clone();
                p.origem = "terminal".into();
                let exc = cer.abrir_excecao(p)?;
                if json {
                    s.out(&dumps(exc.valor()));
                } else {
                    let expira = exc.valor().get("expira").and_then(Value::as_f64).unwrap_or(0.0);
                    s.out(&format!(
                        "exceção {} aberta: {}/{} @{}, uso único, até {}",
                        js_string(exc.valor().get("id")),
                        js_string(exc.valor().get("cabeca")),
                        js_string(exc.valor().get("guarda")),
                        js_string(exc.valor().get("momento")),
                        now_iso(expira)
                    ));
                }
                Ok(OK)
            }
            Some("listar") => {
                let lista = cer.excecoes()?;
                if json {
                    let valores: Vec<&Value> = lista.iter().map(|e| e.valor()).collect();
                    s.out(&dumps(&valores));
                } else {
                    let now = segundos_agora();
                    for e in &lista {
                        let v = e.valor();
                        let expira = v.get("expira").and_then(Value::as_f64).unwrap_or(0.0);
                        let status = if verdade(v.get("usada")) {
                            format!("usada em {}", js_string(v.get("usada")))
                        } else if expira <= now {
                            "expirada".into()
                        } else {
                            "aberta".into()
                        };
                        let alvo = match v.get("alvo") {
                            Some(al) if verdade(Some(al)) && al.as_object().is_some_and(|o| !o.is_empty()) => {
                                py_repr(al)
                            }
                            _ => String::new(),
                        };
                        s.out(&format!(
                            "{} {} {}/{} @{} {alvo}: {}",
                            js_string(v.get("id")),
                            pad(&status, 20),
                            js_string(v.get("cabeca")),
                            js_string(v.get("guarda")),
                            js_string(v.get("momento")),
                            js_string(v.get("motivo")),
                        ));
                    }
                }
                Ok(OK)
            }
            _ => Err(recusa(
                "excecao listar [--json] | excecao abrir --cabeca <c> --guarda <id> --momento <m> --motivo \"<t>\" [--alvo k=v]"
                    .into(),
            )),
        },
        "estado" => {
            s.out(&py_dumps(&Value::Object(cer.estado(true)?), None));
            Ok(OK)
        }
        _ => Err(recusa(format!("comando desconhecido: {cmd}"))),
    }
}

const USO_CHECK: &str = "uso: hades cerberus-check <porta/<guarda> | limiar <guarda> | vigilia/auditoria <guarda> | vigilia/coerencia <guarda>>  (ambiente: CERBERUS_DIR, CERBERUS_SKILL, CERBERUS_MOMENTO, CERBERUS_CTX)";

/// `evaluate` (`src/commands/cerberus-check.ts:12`): a Porta e o Limiar portados; a Vigília responde o uso.
fn avalia(args: &[String], ce: &CheckEnv) -> Option<cerberus_core::check::CheckVerdict> {
    let alvo = args.first().map(String::as_str).unwrap_or("");
    let arg = args.get(1).map(String::as_str);
    let extra = args.len() > 2;
    let mut partes = alvo.split('/');
    let cab = partes.next().unwrap_or("");
    let id = partes.next();
    match cab {
        "porta" => id
            .filter(|i| !i.is_empty())
            .and_then(cerberus_core::porta::guarda)
            .map(|g| g(ce)),
        "limiar" if !(id.is_some_and(|i| !i.is_empty()) || extra) => {
            cerberus_core::limiar::rodar(arg.unwrap_or(""), ce)
        }
        _ => None,
    }
}

/// `cerberusCheckCommand` (`src/commands/cerberus-check.ts:32`): roda uma guarda portada e imprime o veredito numa
/// linha JSON; sem veredito, o uso do alvo (`usageOf`, `:25`) e saída 2.
fn check(args: &[String], ce: &CheckEnv, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let veredito = avalia(args, ce);
    let mut s = Saida { out, err };
    match veredito {
        Some(v) => {
            s.out(&py_dumps(&v.to_json(), None));
            OK
        }
        None => {
            let uso = if args.first().map(String::as_str) == Some("limiar") {
                cerberus_core::limiar::USO
            } else {
                USO_CHECK
            };
            s.warn(uso);
            USO
        }
    }
}

/// `cerberusCommand` (`src/commands/cerberus.ts:200`). Sem a ponte `rpc` (D1): ela recusa toda operação.
fn executar(args: &[String], op: &Opcoes, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let mut s = Saida { out, err };
    let first = args.first().map(String::as_str);
    if matches!(first, None | Some("-h") | Some("--help")) {
        s.out(USAGE);
        return if first.is_none() { USO } else { OK };
    }
    let r = if first == Some("rpc") {
        Err(recusa(format!(
            "operação desconhecida: {}",
            args.get(1).map(String::as_str).unwrap_or("")
        )))
    } else {
        rodar(args, op, &mut s)
    };
    match r {
        Ok(rc) => rc,
        Err(Erro::Recusa(m)) => {
            s.warn(&format!("cerberus: {m}"));
            USO
        }
        Err(e) => {
            s.warn(&format!("cerberus: {e}"));
            FALHA
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let mut op = Opcoes::novo(Env::from_process(), cwd);
    if let Ok(bin) = std::env::current_exe() {
        op = op.com_bin_check(bin);
    }
    let (mut out, mut err) = (std::io::stdout().lock(), std::io::stderr().lock());
    let rc = if args.first().map(String::as_str) == Some("check") {
        let ce = CheckEnv::do_ambiente(std::env::vars().collect(), &op.cwd);
        check(&args[1..], &ce, &mut out, &mut err)
    } else {
        executar(&args, &op, &mut out, &mut err)
    };
    ExitCode::from(u8::try_from(rc).unwrap_or(1))
}

#[cfg(test)]
mod tests;
