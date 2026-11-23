//! Contexto e validação do veredito (`src/lib/cerberus.ts:246-293`), sem disco.

use serde_json::{Value, json};

use crate::Erro;
use crate::modelo::{
    CTX_MAX, CTX_VAL_MAX, Ctx, DETAIL_MAX, EVID_MAX, Resultado, TEXT_MAX, Veredito, clip,
    em_branco, py_repr, tamanho,
};

/// `CTX_KEY` (`src/lib/cerberus.ts:29`): `^[a-z][a-z0-9_]{0,31}$`.
fn chave_valida(key: &str) -> bool {
    let b = key.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

/// `parseCtx` (`src/lib/cerberus.ts:246`): `chave=valor`, o valor pode ter `=`.
pub fn parse_ctx<S: AsRef<str>>(pairs: &[S]) -> Result<Ctx, Erro> {
    let mut ctx = Ctx::new();
    for pair in pairs {
        let pair = pair.as_ref();
        let ok = pair
            .split_once('=')
            .filter(|(key, value)| chave_valida(key) && tamanho(value) <= CTX_VAL_MAX);
        let Some((key, value)) = ok else {
            return Err(Erro::recusa(format!(
                "--ctx {}: chave [a-z][a-z0-9_]{{0,31}}=valor de até {CTX_VAL_MAX} caracteres",
                py_repr(&Value::from(pair))
            )));
        };
        ctx.insert(key.to_owned(), value.to_owned());
    }
    if ctx.len() > CTX_MAX {
        return Err(Erro::recusa(format!(
            "no máximo {CTX_MAX} chaves de contexto"
        )));
    }
    Ok(ctx)
}

fn texto_curto(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str)
        .is_some_and(|s| !em_branco(s) && tamanho(s) <= TEXT_MAX)
}

/// `verdictError` (`src/lib/cerberus.ts:262`): o motivo da recusa do veredito, ou `None`. Veredito sem evidência não
/// vale.
pub fn erro_do_veredito(v: &Value, cab: &str, id: &str, momento: &str) -> Option<String> {
    let Some(v) = v.as_object() else {
        return Some("o veredito não é um objeto JSON".to_owned());
    };
    for (key, want) in [("cabeca", cab), ("guarda", id), ("momento", momento)] {
        if let Some(got) = v.get(key)
            && got.as_str() != Some(want)
        {
            return Some(format!(
                "{key}={} não bate com {}",
                py_repr(got),
                py_repr(&Value::from(want))
            ));
        }
    }
    if v.get("resultado")
        .and_then(Value::as_str)
        .and_then(Resultado::parse)
        .is_none()
    {
        return Some("resultado tem de ser ok, aviso ou barra".to_owned());
    }
    let ev = match v.get("evidencia").and_then(Value::as_array) {
        Some(ev) if !ev.is_empty() => ev,
        _ => return Some("veredito sem evidência".to_owned()),
    };
    if ev.len() > EVID_MAX {
        return Some(format!("evidência com mais de {EVID_MAX} itens"));
    }
    let bom = |item: &Value| {
        item.as_object()
            .is_some_and(|o| texto_curto(o.get("conferido")) && texto_curto(o.get("onde")))
    };
    if !ev.iter().all(bom) {
        return Some(format!(
            "cada evidência precisa de conferido e onde (texto não vazio, até {TEXT_MAX} caracteres)"
        ));
    }
    if let Some(d) = v.get("detalhe")
        && d.as_str().is_none_or(|s| tamanho(s) > DETAIL_MAX)
    {
        return Some(format!("detalhe: texto de até {DETAIL_MAX} caracteres"));
    }
    None
}

/// `refused` (`src/lib/cerberus.ts:286`): o barra que substitui um veredito inválido; a evidência é a do corpo.
pub fn recusado(cab: &str, id: &str, momento: &str, why: &str, onde: &str) -> Veredito {
    let v = json!({
        "cabeca": cab,
        "guarda": id,
        "momento": momento,
        "resultado": "barra",
        "invalido": true,
        "evidencia": [{
            "conferido": clip(&format!("veredito recusado: {why}"), TEXT_MAX),
            "onde": clip(onde, TEXT_MAX),
        }],
        "detalhe": clip(&format!("veredito da guarda recusado ({why})"), DETAIL_MAX),
    });
    match v {
        Value::Object(map) => Veredito(map),
        _ => unreachable!("json! de objeto"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hades_core::jsonfmt::json_compact;

    #[test]
    fn parse_ctx_como_o_golden() {
        let ctx = parse_ctx(&["a=1", "b=x=y"]).unwrap();
        assert_eq!(
            ctx.iter().collect::<Vec<_>>(),
            [
                (&"a".to_owned(), &"1".to_owned()),
                (&"b".to_owned(), &"x=y".to_owned())
            ]
        );
        assert_eq!(
            parse_ctx(&["A=1"]).unwrap_err().to_string(),
            "--ctx 'A=1': chave [a-z][a-z0-9_]{0,31}=valor de até 300 caracteres"
        );
        assert!(parse_ctx(&["sem_igual"]).is_err());
        assert!(parse_ctx(&[format!("k={}", "é".repeat(300))]).is_ok());
        assert!(parse_ctx(&[format!("k={}", "é".repeat(301))]).is_err());
        assert!(parse_ctx(&[format!("{}=1", "a".repeat(33))]).is_err());
        let muitos: Vec<String> = (0..33).map(|i| format!("k{i}=v")).collect();
        assert_eq!(
            parse_ctx(&muitos).unwrap_err().to_string(),
            "no máximo 32 chaves de contexto"
        );
        assert_eq!(parse_ctx(&["a=1", "a=2"]).unwrap()["a"], "2");
    }

    #[test]
    fn erro_do_veredito_na_ordem_do_ts() {
        let erro = |v: Value| erro_do_veredito(&v, "porta", "g", "push");
        assert_eq!(erro(json!([])).unwrap(), "o veredito não é um objeto JSON");
        assert_eq!(
            erro(json!({"cabeca": "z"})).unwrap(),
            "cabeca='z' não bate com 'porta'"
        );
        assert_eq!(
            erro(json!({"guarda": 1})).unwrap(),
            "guarda=1 não bate com 'g'"
        );
        assert_eq!(
            erro(json!({"resultado": "x"})).unwrap(),
            "resultado tem de ser ok, aviso ou barra"
        );
        assert_eq!(
            erro(json!({"resultado": "ok"})).unwrap(),
            "veredito sem evidência"
        );
        assert_eq!(
            erro(json!({"resultado": "ok", "evidencia": []})).unwrap(),
            "veredito sem evidência"
        );
        let item = json!({"conferido": "a", "onde": "b"});
        let vinte_e_um: Vec<Value> = (0..21).map(|_| item.clone()).collect();
        assert_eq!(
            erro(json!({"resultado": "ok", "evidencia": vinte_e_um})).unwrap(),
            "evidência com mais de 20 itens"
        );
        assert_eq!(
            erro(json!({"resultado": "ok", "evidencia": [{"conferido": " ", "onde": "b"}]}))
                .unwrap(),
            "cada evidência precisa de conferido e onde (texto não vazio, até 500 caracteres)"
        );
        assert_eq!(
            erro(json!({"resultado": "ok", "evidencia": [item], "detalhe": null})).unwrap(),
            "detalhe: texto de até 1000 caracteres"
        );
        assert_eq!(
            erro(
                json!({"cabeca": "porta", "resultado": "barra", "evidencia": [item], "detalhe": "d"})
            ),
            None
        );
    }

    #[test]
    fn recusado_na_ordem_do_ts() {
        let v = recusado("porta", "g", "push", &"x".repeat(600), "aqui");
        assert_eq!(v.resultado(), Some(Resultado::Barra));
        let texto = json_compact(&Value::Object(v.campos().clone()));
        assert!(texto.starts_with(
            r#"{"cabeca":"porta","guarda":"g","momento":"push","resultado":"barra","invalido":true,"evidencia":[{"conferido":"veredito recusado: xxx"#
        ));
        let conferido = v.campos()["evidencia"][0]["conferido"].as_str().unwrap();
        assert_eq!(tamanho(conferido), TEXT_MAX);
        assert_eq!(v.campos()["evidencia"][0]["onde"], "aqui");
        assert!(v.campos()["detalhe"].as_str().unwrap().ends_with("xxx)"));
    }
}
