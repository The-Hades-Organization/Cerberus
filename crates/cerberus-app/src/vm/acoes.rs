//! View-model das telas de ação: Exceções e Testar guarda.

use cerberus_core::modelo::{
    Cabeca, EXC_VALIDITY_MIN_CHARS, Excecao, Resultado, TEXT_MAX, Tipo, Veredito, now_iso, py_repr,
    tamanho,
};
use cerberus_core::registro::Guarda;
use hades_ui::Tone;
use serde_json::Value;

use super::{data_hora, mascarar};

pub const COLUNAS_EXCECOES: [&str; 5] = ["Id", "Alvo", "Prazo", "Motivo", "Estado"];

/// Uma exceção como a lista mostra.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinhaExcecao {
    pub id: String,
    pub alvo: String,
    pub prazo: String,
    pub motivo: String,
    pub estado: String,
    pub tom: Tone,
}

impl LinhaExcecao {
    pub fn celulas(&self) -> [String; 5] {
        [
            self.id.clone(),
            self.alvo.clone(),
            self.prazo.clone(),
            self.motivo.clone(),
            self.estado.clone(),
        ]
    }
}

/// Verdade no sentido do JS, como o `excecao listar` do CLI testa `usada` e `alvo`.
fn verdade(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn texto_de(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        None | Some(Value::Null) => String::new(),
        Some(outro) => outro.to_string(),
    }
}

/// `excecao listar` do CLI (`crates/cerberus-cli/src/main.rs:374`): usada → "usada em <avaliação>", prazo vencido →
/// "expirada", senão "aberta"; o alvo sai como o Python o imprime, só quando não é vazio.
pub fn linha_excecao(e: &Excecao, agora_s: f64) -> LinhaExcecao {
    let v = e.valor();
    let expira = v.get("expira").and_then(Value::as_f64).unwrap_or(0.0);
    let (estado, tom) = if verdade(v.get("usada")) {
        (
            format!("usada em {}", texto_de(v.get("usada"))),
            Tone::Neutral,
        )
    } else if expira <= agora_s {
        ("expirada".to_owned(), Tone::Neutral)
    } else {
        ("aberta".to_owned(), Tone::Warn)
    };
    let alvo = match v.get("alvo") {
        Some(al) if al.as_object().is_some_and(|o| !o.is_empty()) => format!(" {}", py_repr(al)),
        _ => String::new(),
    };
    LinhaExcecao {
        id: texto_de(v.get("id")),
        alvo: format!(
            "{}/{} @{}{alvo}",
            texto_de(v.get("cabeca")),
            texto_de(v.get("guarda")),
            texto_de(v.get("momento")),
        ),
        prazo: data_hora(&now_iso(expira)),
        motivo: texto_de(v.get("motivo")),
        estado,
        tom,
    }
}

/// Quantas exceções ainda valem (nem usadas nem vencidas).
pub fn abertas(excecoes: &[Excecao], agora_s: f64) -> usize {
    excecoes
        .iter()
        .filter(|e| linha_excecao(e, agora_s).estado == "aberta")
        .count()
}

/// A mesma regra do `openException` do core: motivo aparado com 10 a 500 caracteres.
pub fn erro_do_motivo(texto: &str) -> Option<String> {
    let n = tamanho(texto.trim());
    (n < EXC_VALIDITY_MIN_CHARS || tamanho(texto) > TEXT_MAX)
        .then(|| format!("motivo obrigatório ({EXC_VALIDITY_MIN_CHARS} a {TEXT_MAX} caracteres)"))
}

/// Validade em horas, inteira de 1 a `max` (o `HADES_CERBERUS_EXC_TTL_H` efetivo).
pub fn ttl_em_horas(texto: &str, max: i64) -> Result<i64, String> {
    let t = texto.trim();
    match t.parse::<i64>() {
        Ok(h) if (1..=max).contains(&h) && t.bytes().all(|b| b.is_ascii_digit()) => Ok(h),
        _ => Err(format!("validade: inteiro de 1 a {max} horas")),
    }
}

/// Pares `chave=valor` do alvo, separados por espaço; o core valida cada um.
pub fn pares_do_alvo(texto: &str) -> Vec<String> {
    texto.split_whitespace().map(str::to_owned).collect()
}

/// Cabeça, guarda e momento escolhidos num formulário.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Escolha {
    pub cabeca: Cabeca,
    pub guarda: Option<String>,
    pub momento: Option<String>,
}

impl Default for Escolha {
    fn default() -> Self {
        Self {
            cabeca: Cabeca::Porta,
            guarda: None,
            momento: None,
        }
    }
}

impl Escolha {
    pub fn de(cabeca: &str, guarda: &str, momento: &str) -> Option<Escolha> {
        Some(Escolha {
            cabeca: Cabeca::parse(cabeca)?,
            guarda: (!guarda.is_empty()).then(|| guarda.to_owned()),
            momento: (!momento.is_empty()).then(|| momento.to_owned()),
        })
    }

    /// Troca a cabeça e esquece guarda e momento, que eram da outra.
    pub fn trocar_cabeca(&mut self, cabeca: Cabeca) {
        if self.cabeca != cabeca {
            *self = Escolha {
                cabeca,
                ..Escolha::default()
            };
        }
    }

    /// Troca a guarda e fica com o primeiro momento dela se o atual não serve.
    pub fn trocar_guarda(&mut self, guarda: &Guarda) {
        self.guarda = Some(guarda.id.clone());
        let serve = self
            .momento
            .as_deref()
            .is_some_and(|m| guarda.momentos.iter().any(|g| g.as_str() == m));
        if !serve {
            self.momento = guarda.momentos.first().map(|m| m.as_str().to_owned());
        }
    }

    pub fn completa(&self) -> bool {
        self.guarda.is_some() && self.momento.is_some()
    }
}

/// As guardas que o formulário oferece para a cabeça: todas, ou só as exec no Testar guarda.
pub fn guardas_da_escolha(guardas: &[Guarda], cabeca: Cabeca, so_exec: bool) -> Vec<&Guarda> {
    guardas
        .iter()
        .filter(|g| g.cabeca == cabeca && (!so_exec || g.tipo == Tipo::Exec))
        .collect()
}

/// Os momentos da guarda escolhida.
pub fn momentos_da_escolha(guardas: &[Guarda], e: &Escolha) -> Vec<String> {
    guardas
        .iter()
        .find(|g| g.cabeca == e.cabeca && Some(&g.id) == e.guarda.as_ref())
        .map(|g| g.momentos.iter().map(|m| m.as_str().to_owned()).collect())
        .unwrap_or_default()
}

/// O resultado de um ensaio como a tela mostra: rótulo, tom e as linhas da evidência já mascaradas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultadoEnsaio {
    pub rotulo: &'static str,
    pub tom: Tone,
    pub evidencia: Vec<String>,
    pub detalhe: Option<String>,
}

pub fn resultado_do_ensaio(v: &Veredito) -> ResultadoEnsaio {
    let (rotulo, tom) = match v.resultado() {
        Some(Resultado::Ok) => ("passa", Tone::Ok),
        Some(Resultado::Aviso) => ("aviso", Tone::Warn),
        _ => ("barra", Tone::Bad),
    };
    let evidencia = v
        .campos()
        .get("evidencia")
        .and_then(Value::as_array)
        .map(|itens| {
            itens
                .iter()
                .map(|i| {
                    let onde = texto_de(i.get("onde"));
                    let conferido = mascarar(&texto_de(i.get("conferido")));
                    if onde.is_empty() {
                        conferido
                    } else {
                        format!("{onde}: {conferido}")
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let detalhe = v
        .campos()
        .get("detalhe")
        .and_then(Value::as_str)
        .filter(|d| !d.is_empty())
        .map(mascarar);
    ResultadoEnsaio {
        rotulo,
        tom,
        evidencia,
        detalhe,
    }
}
