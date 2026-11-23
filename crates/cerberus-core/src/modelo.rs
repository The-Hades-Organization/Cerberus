//! Vocabulário do Cerberus (`src/lib/cerberus.ts:16-89`) e os ajustes de texto e número que a paridade com o TS pede.

use std::fmt;

use hades_core::jsonfmt::json_compact;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

macro_rules! texto_fixo {
    ($(#[$meta:meta])* $nome:ident { $($variante:ident = $texto:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[non_exhaustive]
        pub enum $nome {
            $(#[serde(rename = $texto)] $variante),+
        }

        impl $nome {
            /// Todos, na ordem do TS.
            pub const TODOS: &'static [$nome] = &[$($nome::$variante),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $($nome::$variante => $texto),+
                }
            }

            pub fn parse(texto: &str) -> Option<Self> {
                match texto {
                    $($texto => Some($nome::$variante),)+
                    _ => None,
                }
            }

            /// Os textos separados por `", "`, como o `.join(", ")` das mensagens do TS.
            pub fn lista() -> String {
                Self::TODOS.iter().map(|x| x.as_str()).collect::<Vec<_>>().join(", ")
            }
        }

        impl fmt::Display for $nome {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

texto_fixo!(
    /// `CABECAS` (`src/lib/cerberus.ts:16`).
    Cabeca { Porta = "porta", Limiar = "limiar", Vigilia = "vigilia" }
);

impl Cabeca {
    /// `NOMES` (`src/lib/cerberus.ts:18`).
    pub fn nome(self) -> &'static str {
        match self {
            Cabeca::Porta => "Porta",
            Cabeca::Limiar => "Limiar",
            Cabeca::Vigilia => "Vigília",
        }
    }
}

texto_fixo!(
    /// `MOMENTOS` (`src/lib/cerberus.ts:19`).
    Momento {
        Done = "done",
        Push = "push",
        Release = "release",
        Plugin = "plugin",
        Consolidacao = "consolidacao",
        Continuo = "continuo",
        Publicacao = "publicacao",
        Comentario = "comentario",
        Artefato = "artefato",
        Tunel = "tunel",
    }
);

texto_fixo!(
    /// `RESULTADOS` (`src/lib/cerberus.ts:22`): o que uma guarda pode concluir.
    Resultado { Ok = "ok", Aviso = "aviso", Barra = "barra" }
);

texto_fixo!(
    /// `EFEITOS` (`src/lib/cerberus.ts:23`): o resultado depois do modo da cabeça e das exceções.
    Efeito { Ok = "ok", Aviso = "aviso", Barra = "barra", Liberado = "liberado" }
);

texto_fixo!(
    /// `MODOS` (`src/lib/cerberus.ts:24`).
    Modo { Off = "off", Avisa = "avisa", Barra = "barra" }
);

texto_fixo!(
    /// `TIPOS` (`src/lib/cerberus.ts:25`).
    Tipo { Exec = "exec", Emitida = "emitida", Regra = "regra" }
);

/// Limites de `src/lib/cerberus.ts:30-41`.
pub const CTX_VAL_MAX: usize = 300;
pub const CTX_MAX: usize = 32;
pub const EVID_MAX: usize = 20;
pub const TEXT_MAX: usize = 500;
pub const DETAIL_MAX: usize = 1000;
/// Caracteres da saída de uma guarda lidos pelo corpo; o resto não é veredito.
pub const OUT_MAX: usize = 65536;
/// Teto do que o subprocesso pode escrever antes de ser cortado.
pub const OUT_BUFFER: usize = 64 * 1024 * 1024;
pub const TIMEOUT_MAX: u64 = 600;
/// Vereditos que o estado do painel devolve (os mais novos).
pub const PAGE_MAX: usize = 500;
pub const EXC_LIST_MAX: usize = 100;
pub const EXC_VALIDITY_MIN_CHARS: usize = 10;

/// `Dict` (`src/lib/cerberus.ts:43`): objeto JSON com a ordem de chaves preservada.
pub type Dict = Map<String, Value>;

/// Contexto de uma avaliação (`--ctx chave=valor`), na ordem em que veio.
pub type Ctx = IndexMap<String, String>;

/// `Veredito` (`src/lib/cerberus.ts:53`): o que a guarda imprime ou o corpo monta e, depois do efeito, o registro do
/// histórico. Fica como objeto para as chaves e a ordem saírem como o TS grava.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Veredito(pub Dict);

impl Veredito {
    fn texto(&self, chave: &str) -> Option<&str> {
        self.0.get(chave).and_then(Value::as_str)
    }

    pub fn cabeca(&self) -> Option<&str> {
        self.texto("cabeca")
    }

    pub fn guarda(&self) -> Option<&str> {
        self.texto("guarda")
    }

    pub fn momento(&self) -> Option<&str> {
        self.texto("momento")
    }

    pub fn resultado(&self) -> Option<Resultado> {
        self.texto("resultado").and_then(Resultado::parse)
    }

    pub fn campos(&self) -> &Dict {
        &self.0
    }

    pub fn into_dict(self) -> Dict {
        self.0
    }
}

/// `Excecao` (`src/lib/cerberus.ts:60`). A lista do disco pode trazer item que não é objeto; ele fica como veio para
/// regravar a lista sem perdê-lo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Excecao(pub Value);

impl Excecao {
    pub fn campos(&self) -> Option<&Dict> {
        self.0.as_object()
    }

    pub fn texto(&self, chave: &str) -> Option<&str> {
        self.campos()?.get(chave).and_then(Value::as_str)
    }

    pub fn id(&self) -> Option<&str> {
        self.texto("id")
    }

    pub fn valor(&self) -> &Value {
        &self.0
    }
}

/// `repr()` do Python (`src/lib/pyjson.ts:52`): aspas simples (duplas se o texto tem aspas simples e não duplas),
/// `None`/`True`/`False`, listas e dicts. Diferente do `profile::py_repr` do core, escapa controle como `\xNN`.
pub fn py_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(b) => if *b { "True" } else { "False" }.to_owned(),
        Value::Number(_) => json_compact(value),
        Value::String(s) => repr_texto(s),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(py_repr).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!("{}: {}", repr_texto(k), py_repr(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// `reprString` (`src/lib/pyjson.ts:63`).
pub fn repr_texto(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for ch in text.chars() {
        match ch {
            c if c == quote || c == '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Espaço do `\s` e do `trim` do JS: inclui NBSP e BOM, não inclui U+0085.
pub fn js_space(c: char) -> bool {
    c == '\u{feff}' || (c.is_whitespace() && c != '\u{85}')
}

/// `String.prototype.trim` do JS.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(js_space)
}

/// `blank` (`src/lib/cerberus.ts:88`).
pub fn em_branco(text: &str) -> bool {
    js_trim(text).is_empty()
}

/// `charLength` (`src/lib/cerberus.ts:89`): conta pontos de código, não bytes.
pub fn tamanho(text: &str) -> usize {
    text.chars().count()
}

/// `clip` (`src/lib/cerberus.ts:283`): os primeiros `max` pontos de código.
pub fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// `nowIso` (`src/lib/cerberus.ts:159`): segundos (com fração) para `AAAA-MM-DDTHH:MM:SSZ`.
pub fn now_iso(seconds: f64) -> String {
    let iso = hades_core::journal::iso_ms((seconds * 1000.0).trunc() as i64);
    format!("{}Z", &iso[..19])
}

/// `nowSeconds` (`src/lib/cerberus.ts:163`): `Date.now() / 1000`, com fração.
pub fn now_secs() -> f64 {
    hades_core::journal::now_ms() as f64 / 1000.0
}

const JS_INT_MAX: f64 = 9_007_199_254_740_992.0;

/// Número como o JS o grava: inteiro sem casa decimal (`1761480000`), fração como veio. O `py_dumps` do core trata
/// todo `f64` como float do Python (`1761480000.0`), e o TS só faz isso para `PyFloat`.
pub fn js_num(value: f64) -> Value {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < JS_INT_MAX {
        Value::from(value as i64)
    } else {
        Number::from_f64(value).map_or(Value::Null, Value::Number)
    }
}

/// O que o `JSON.parse` do TS faz com os números de um arquivo: `1.0` vira `1` (todo número é `f64` no JS, e o
/// inteiro sai sem casa decimal quando regravado).
pub fn js_norm(value: Value) -> Value {
    match value {
        Value::Number(n) if n.is_f64() => n.as_f64().map_or(Value::Number(n), js_num),
        Value::Array(items) => Value::Array(items.into_iter().map(js_norm).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, js_norm(v))).collect())
        }
        other => other,
    }
}

/// `String(x)` do JS para o que vem de um registro: ausente é `"undefined"`.
pub(crate) fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(Value::Null) => "null".to_owned(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                Value::Null => String::new(),
                other => js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_owned(),
        Some(other) => json_compact(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn listas_na_ordem_do_ts() {
        assert_eq!(Cabeca::lista(), "porta, limiar, vigilia");
        assert_eq!(
            Momento::lista(),
            "done, push, release, plugin, consolidacao, continuo, publicacao, comentario, artefato, tunel"
        );
        assert_eq!(Tipo::lista(), "exec, emitida, regra");
        assert_eq!(Cabeca::Vigilia.nome(), "Vigília");
        assert_eq!(
            (EVID_MAX, TEXT_MAX, DETAIL_MAX, CTX_MAX),
            (20, 500, 1000, 32)
        );
    }

    #[test]
    fn serde_em_minusculas() {
        assert_eq!(
            serde_json::to_value(Efeito::Liberado).unwrap(),
            json!("liberado")
        );
        assert_eq!(
            serde_json::from_value::<Momento>(json!("tunel")).unwrap(),
            Momento::Tunel
        );
    }

    #[test]
    fn repr_como_o_python() {
        assert_eq!(py_repr(&json!("x'y")), "\"x'y\"");
        assert_eq!(py_repr(&json!("A=1")), "'A=1'");
        assert_eq!(py_repr(&json!("a'b\"c")), "'a\\'b\"c'");
        assert_eq!(py_repr(&json!("\u{1}\u{7f}\n")), "'\\x01\\x7f\\n'");
        assert_eq!(
            py_repr(&json!([null, true, 3, {"k": 1.5}])),
            "[None, True, 3, {'k': 1.5}]"
        );
    }

    #[test]
    fn trim_do_js() {
        assert_eq!(js_trim("\u{feff}\u{a0} a \n"), "a");
        assert_eq!(js_trim("\u{85}a"), "\u{85}a");
        assert!(em_branco(" \t "));
    }

    #[test]
    fn instante_e_numeros() {
        assert_eq!(now_iso(1_761_480_000.0), "2025-10-26T12:00:00Z");
        assert_eq!(now_iso(1_761_480_000.999), "2025-10-26T12:00:00Z");
        assert_eq!(js_num(3.0), json!(3));
        assert_eq!(js_num(1.5), json!(1.5));
        assert_eq!(
            js_norm(json!({"a": [1761480000.0, 2.5], "b": "x"})),
            json!({"a": [1761480000, 2.5], "b": "x"})
        );
    }

    #[test]
    fn string_do_js() {
        assert_eq!(js_string(None), "undefined");
        assert_eq!(js_string(Some(&json!(3.0))), "3");
        assert_eq!(js_string(Some(&json!([1, null, "a"]))), "1,,a");
    }
}
