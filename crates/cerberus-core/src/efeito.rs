//! Efeito de um veredito sob o modo da cabeça e as exceções abertas (`src/lib/cerberus.ts:295-314`).

use serde_json::Value;

use crate::modelo::{Ctx, Efeito, Excecao, Modo, Resultado, Veredito};

/// Falsy do JS para o que vem de um arquivo.
fn falsy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => !b,
        Some(Value::Number(n)) => n.as_f64() == Some(0.0),
        Some(Value::String(s)) => s.is_empty(),
        Some(_) => false,
    }
}

/// `exceptionMatches` (`src/lib/cerberus.ts:295`): de uso único, dentro do prazo, da mesma guarda e momento, e cada
/// chave do alvo igual à do contexto (`null` no alvo exige a chave ausente).
pub fn casa(exc: &Excecao, v: &Veredito, ctx: &Ctx, now: f64) -> bool {
    let Some(e) = exc.campos() else {
        return false;
    };
    let expira = e.get("expira").and_then(Value::as_f64).unwrap_or(0.0);
    let igual = |k: &str| {
        e.get(k).and_then(Value::as_str) == v.campos().get(k).and_then(Value::as_str)
            && e.get(k).is_some_and(Value::is_string)
    };
    let alvo_ok = match e.get("alvo") {
        Some(Value::Object(alvo)) => alvo.iter().all(|(k, want)| match want {
            Value::Null => !ctx.contains_key(k),
            Value::String(s) => ctx.get(k) == Some(s),
            _ => false,
        }),
        _ => true,
    };
    falsy(e.get("usada"))
        && expira > now
        && igual("cabeca")
        && igual("guarda")
        && igual("momento")
        && alvo_ok
}

/// `effect` (`src/lib/cerberus.ts:303`): o efeito e o índice da exceção que liberou, se alguma.
pub fn efeito(
    v: &Veredito,
    modo: Modo,
    excs: &[Excecao],
    ctx: &Ctx,
    now: f64,
) -> (Efeito, Option<usize>) {
    match v.resultado() {
        Some(Resultado::Barra) => {}
        Some(Resultado::Aviso) => return (Efeito::Aviso, None),
        _ => return (Efeito::Ok, None),
    }
    if modo == Modo::Avisa {
        return (Efeito::Aviso, None);
    }
    match excs.iter().position(|e| casa(e, v, ctx, now)) {
        Some(i) => (Efeito::Liberado, Some(i)),
        None => (Efeito::Barra, None),
    }
}

/// `overall` (`src/lib/cerberus.ts:310`): barra se algum barra; aviso se algum avisou ou foi liberado; senão ok.
pub fn geral(efeitos: &[Efeito]) -> Resultado {
    if efeitos.contains(&Efeito::Barra) {
        Resultado::Barra
    } else if efeitos
        .iter()
        .any(|e| matches!(e, Efeito::Aviso | Efeito::Liberado))
    {
        Resultado::Aviso
    } else {
        Resultado::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn veredito(resultado: &str) -> Veredito {
        match json!({"cabeca": "porta", "guarda": "g", "momento": "push", "resultado": resultado}) {
            Value::Object(m) => Veredito(m),
            _ => unreachable!(),
        }
    }

    fn exc(extra: Value) -> Excecao {
        let mut base = json!({"id": "e", "cabeca": "porta", "guarda": "g", "momento": "push",
            "alvo": {"pr": "1"}, "expira": 2000, "usada": null});
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        Excecao(base)
    }

    fn ctx(pares: &[(&str, &str)]) -> Ctx {
        pares
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    }

    #[test]
    fn ok_e_aviso_passam_direto() {
        let e = [exc(json!({}))];
        assert_eq!(
            efeito(&veredito("ok"), Modo::Barra, &e, &ctx(&[]), 1000.0),
            (Efeito::Ok, None)
        );
        assert_eq!(
            efeito(&veredito("aviso"), Modo::Barra, &e, &ctx(&[]), 1000.0),
            (Efeito::Aviso, None)
        );
        assert_eq!(
            efeito(&veredito("barra"), Modo::Avisa, &e, &ctx(&[]), 1000.0),
            (Efeito::Aviso, None)
        );
    }

    #[test]
    fn excecao_libera_so_quando_casa() {
        let v = veredito("barra");
        let certo = ctx(&[("pr", "1"), ("outra", "x")]);
        let lista = [exc(json!({"guarda": "outra"})), exc(json!({}))];
        assert_eq!(
            efeito(&v, Modo::Barra, &lista, &certo, 1000.0),
            (Efeito::Liberado, Some(1))
        );
        // alvo errado
        assert_eq!(
            efeito(&v, Modo::Barra, &lista, &ctx(&[("pr", "2")]), 1000.0),
            (Efeito::Barra, None)
        );
        // já usada
        let usada = [exc(json!({"usada": "2025-10-26T12:00:00Z"}))];
        assert_eq!(
            efeito(&v, Modo::Barra, &usada, &certo, 1000.0),
            (Efeito::Barra, None)
        );
        // vencida, ou sem prazo numérico
        assert_eq!(
            efeito(&v, Modo::Barra, &lista, &certo, 2000.0),
            (Efeito::Barra, None)
        );
        let sem_prazo = [exc(json!({"expira": "3000"}))];
        assert_eq!(
            efeito(&v, Modo::Barra, &sem_prazo, &certo, 1000.0),
            (Efeito::Barra, None)
        );
        // sem alvo libera qualquer contexto
        let sem_alvo = [exc(json!({"alvo": {}}))];
        assert_eq!(
            efeito(&v, Modo::Barra, &sem_alvo, &ctx(&[]), 1000.0),
            (Efeito::Liberado, Some(0))
        );
        // alvo null exige a chave ausente; item que não é objeto nunca casa
        let nulo = [Excecao(json!(3)), exc(json!({"alvo": {"pr": null}}))];
        assert_eq!(
            efeito(&v, Modo::Barra, &nulo, &ctx(&[]), 1000.0),
            (Efeito::Liberado, Some(1))
        );
        assert_eq!(
            efeito(&v, Modo::Barra, &nulo, &certo, 1000.0),
            (Efeito::Barra, None)
        );
        let numero = [exc(json!({"alvo": {"pr": 1}}))];
        assert_eq!(
            efeito(&v, Modo::Barra, &numero, &certo, 1000.0),
            (Efeito::Barra, None)
        );
    }

    #[test]
    fn geral_como_o_ts() {
        assert_eq!(geral(&[]), Resultado::Ok);
        assert_eq!(geral(&[Efeito::Ok, Efeito::Liberado]), Resultado::Aviso);
        assert_eq!(geral(&[Efeito::Aviso, Efeito::Ok]), Resultado::Aviso);
        assert_eq!(geral(&[Efeito::Liberado, Efeito::Barra]), Resultado::Barra);
    }
}
