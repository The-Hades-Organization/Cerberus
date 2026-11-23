//! As operações do Cerberus (`src/lib/cerberus.ts:472-579`) atrás da fachada [`Cerberus`]: avaliar um momento,
//! registrar o veredito de uma guarda emitida, ler o histórico, abrir exceção e montar o estado do painel.

use std::sync::Arc;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::config::{Cfg, Modos, cfg_int, config, modos};
use crate::ctx::{erro_do_veredito, parse_ctx};
use crate::efeito::{efeito, geral};
use crate::executor::{Executor, ExecutorProcesso, Lugar, rodar_guarda};
use crate::modelo::{
    Cabeca, Ctx, Dict, EXC_LIST_MAX, Efeito, Excecao, Momento, PAGE_MAX, Resultado, TIMEOUT_MAX,
    Tipo, js_string, now_iso, now_secs,
};
use crate::registro::{Guarda, Registro};
use crate::store::{NovaExcecao, Store, id_aleatorio};
use crate::{Erro, Opcoes};

/// `Avaliacao` (`src/lib/cerberus.ts:474`): o resultado de `avaliar`, com as chaves na ordem do TS.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Avaliacao {
    pub avaliacao: String,
    pub momento: String,
    pub resultado: Resultado,
    pub vereditos: Vec<Dict>,
    pub barras: usize,
    pub avisos: usize,
    /// Cabeças em `off` entre as avaliadas.
    pub ignoradas: Vec<String>,
}

impl Avaliacao {
    /// A CLI sai 1 quando isto é verdade.
    pub fn barrou(&self) -> bool {
        self.resultado == Resultado::Barra
    }
}

/// O que `avaliar` recebe além do momento (`opts` de `evaluate`, `src/lib/cerberus.ts:484`).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Entrada {
    pub ctx: Ctx,
    /// Pasta onde as guardas rodam; ausente, [`Opcoes::cwd`].
    pub cwd: Option<String>,
    /// Só esta cabeça; vazia ou ausente, todas.
    pub cabeca: Option<String>,
    /// Catálogo dado; ausente, o do ambiente.
    pub cfg: Option<Cfg>,
    pub now: Option<f64>,
}

/// O veredito de uma guarda emitida (`RecordInput`, `src/lib/cerberus.ts:523`).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct PedidoRegistro {
    pub cabeca: String,
    pub guarda: String,
    pub momento: String,
    pub resultado: String,
    pub evidencia: Value,
    pub detalhe: Option<String>,
    pub efeito: Option<String>,
    pub ctx: Ctx,
    /// Gravado cru em `dir`; ausente ou vazio, [`Opcoes::cwd`].
    pub cwd: Option<String>,
    pub cfg: Option<Cfg>,
    pub now: Option<f64>,
}

/// Exceção pedida pelo dono (`excecao abrir`, `src/commands/cerberus.ts:122`).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct PedidoExcecao {
    pub cabeca: String,
    pub guarda: String,
    pub momento: String,
    pub motivo: String,
    /// Pares `chave=valor`.
    pub alvo: Vec<String>,
    pub por: Option<String>,
    pub origem: String,
    /// Validade; ausente, `HADES_CERBERUS_EXC_TTL_H` horas.
    pub ttl_s: Option<i64>,
    pub now: Option<f64>,
}

/// A fachada: as operações do Cerberus sobre um ambiente e um executor de guardas.
#[derive(Clone)]
pub struct Cerberus {
    op: Opcoes,
    ex: Arc<dyn Executor>,
}

fn ctx_obj(ctx: &Ctx) -> Value {
    Value::Object(
        ctx.iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect(),
    )
}

impl Cerberus {
    /// Não toca o disco: a pasta de estado é criada na primeira operação que grava ou lê.
    pub fn abrir(op: Opcoes) -> Self {
        Self {
            op,
            ex: Arc::new(ExecutorProcesso),
        }
    }

    pub fn com_executor(mut self, ex: Arc<dyn Executor>) -> Self {
        self.ex = ex;
        self
    }

    pub fn opcoes(&self) -> &Opcoes {
        &self.op
    }

    /// `config` (`src/lib/cerberus.ts:97`).
    pub fn config(&self, com_env: bool) -> Cfg {
        config(&self.op, com_env)
    }

    /// `registry` (`src/lib/cerberus.ts:213`).
    pub fn registro(&self) -> Registro {
        Registro::carregar(&self.op.skill())
    }

    /// As guardas da cabeça dada (todas se ausente ou vazia) e os erros do registro.
    pub fn guardas(&self, cabeca: Option<&str>) -> (Vec<Guarda>, Vec<String>) {
        let r = self.registro();
        let cab = cabeca.filter(|c| !c.is_empty());
        let shown = r
            .guardas
            .into_iter()
            .filter(|g| cab.is_none_or(|c| g.cabeca.as_str() == c))
            .collect();
        (shown, r.erros)
    }

    /// `modes(config(env))`.
    pub fn modos(&self) -> Result<Modos, Erro> {
        modos(&self.config(true))
    }

    fn store(&self) -> Result<Store, Erro> {
        Ok(Store::de(&self.op)?)
    }

    fn cwd_texto(&self) -> String {
        self.op.cwd.to_string_lossy().into_owned()
    }

    /// `evaluate` (`src/lib/cerberus.ts:484`): as guardas exec do momento rodam em sequência, fora do lock; o efeito
    /// de cada veredito e o uso das exceções são decididos sob o flock, e o histórico é anexado depois.
    pub fn avaliar(&self, momento: &str, entrada: Entrada) -> Result<Avaliacao, Erro> {
        let cfg = entrada.cfg.unwrap_or_else(|| self.config(true));
        let now = entrada.now.unwrap_or_else(now_secs);
        if Momento::parse(momento).is_none() {
            return Err(Erro::recusa(format!(
                "momento inválido: {momento} ({})",
                Momento::lista()
            )));
        }
        let only = entrada.cabeca.filter(|c| !c.is_empty());
        if let Some(c) = &only
            && Cabeca::parse(c).is_none()
        {
            return Err(Erro::recusa(format!("cabeça inválida: {c}")));
        }
        let reg = self.registro();
        if !reg.erros.is_empty() {
            return Err(Erro::recusa(format!(
                "registro do Cerberus inválido: {}",
                reg.erros.join(" | ")
            )));
        }
        let mode = modos(&cfg)?;
        let timeout_s = cfg_int(&cfg, "HADES_CERBERUS_TIMEOUT_S", 1, TIMEOUT_MAX as i64)? as u64;
        let heads: Vec<Cabeca> = Cabeca::TODOS
            .iter()
            .copied()
            .filter(|c| only.as_deref().is_none_or(|o| c.as_str() == o))
            .collect();
        let cwd = entrada.cwd.unwrap_or_else(|| self.cwd_texto());
        let skill = self.op.skill();
        let lugar = Lugar {
            skill: &skill,
            cwd: std::path::Path::new(&cwd),
            bin_check: self.op.bin_check.as_deref(),
        };
        let aid = id_aleatorio()?;
        let verdicts: Vec<_> = reg
            .guardas
            .iter()
            .filter(|g| {
                g.tipo == Tipo::Exec
                    && g.momentos.iter().any(|m| m.as_str() == momento)
                    && heads.contains(&g.cabeca)
                    && mode.de(g.cabeca) != crate::modelo::Modo::Off
            })
            .map(|g| {
                rodar_guarda(
                    self.ex.as_ref(),
                    g,
                    momento,
                    &entrada.ctx,
                    &lugar,
                    timeout_s,
                )
            })
            .collect();

        let store = self.store()?;
        let ctx = ctx_obj(&entrada.ctx);
        let records = store.travado(|| -> Result<Vec<Dict>, Erro> {
            let mut excs = store.carregar_excecoes()?;
            let mut records = Vec::new();
            let mut usou = false;
            for v in &verdicts {
                let cab = v.cabeca().and_then(Cabeca::parse).unwrap_or(Cabeca::Porta);
                let modo = mode.de(cab);
                let (ef, idx) = efeito(v, modo, &excs, &entrada.ctx, now);
                let mut rec = Map::new();
                rec.insert("ts".into(), now_iso(now).into());
                rec.insert("avaliacao".into(), aid.clone().into());
                for (k, val) in v.campos() {
                    rec.insert(k.clone(), val.clone());
                }
                rec.insert("modo".into(), modo.as_str().into());
                rec.insert("efeito".into(), ef.as_str().into());
                rec.insert("origem".into(), "avaliar".into());
                rec.insert("ctx".into(), ctx.clone());
                rec.insert("dir".into(), cwd.clone().into());
                if let Some(Value::Object(e)) = idx.map(|i| &mut excs[i].0) {
                    e.insert("usada".into(), aid.clone().into());
                    e.insert("usadaEm".into(), now_iso(now).into());
                    let resumo: Map<String, Value> = ["id", "motivo", "por", "aberta"]
                        .iter()
                        .filter_map(|k| e.get(*k).map(|v| ((*k).to_owned(), v.clone())))
                        .collect();
                    rec.insert("excecao".into(), Value::Object(resumo));
                    usou = true;
                }
                records.push(rec);
            }
            if usou {
                store.gravar_excecoes(&excs)?;
            }
            Ok(records)
        })??;
        store.anexar_historico(&records, &cfg, &self.op)?;

        let efeitos: Vec<Efeito> = records
            .iter()
            .filter_map(|r| {
                r.get("efeito")
                    .and_then(Value::as_str)
                    .and_then(Efeito::parse)
            })
            .collect();
        Ok(Avaliacao {
            avaliacao: aid,
            momento: momento.to_owned(),
            resultado: geral(&efeitos),
            barras: efeitos.iter().filter(|e| **e == Efeito::Barra).count(),
            avisos: efeitos
                .iter()
                .filter(|e| matches!(e, Efeito::Aviso | Efeito::Liberado))
                .count(),
            ignoradas: heads
                .iter()
                .filter(|c| mode.de(**c) == crate::modelo::Modo::Off)
                .map(|c| c.as_str().to_owned())
                .collect(),
            vereditos: records,
        })
    }

    /// `record` (`src/lib/cerberus.ts:533`): o script de uma guarda emitida decidiu; o corpo valida e grava.
    pub fn registrar(&self, p: PedidoRegistro) -> Result<Dict, Erro> {
        let cfg = p.cfg.unwrap_or_else(|| self.config(true));
        let now = p.now.unwrap_or_else(now_secs);
        let (cab, id, momento) = (p.cabeca.as_str(), p.guarda.as_str(), p.momento.as_str());
        let reg = self.registro();
        let g = reg
            .achar(cab, id)
            .filter(|g| g.tipo == Tipo::Emitida)
            .ok_or_else(|| {
                Erro::recusa(format!(
                    "{cab}/{id} não está registrada como guarda emitida"
                ))
            })?;
        if !g.momentos.iter().any(|m| m.as_str() == momento) {
            return Err(Erro::recusa(format!(
                "{cab}/{id} não é registrada no momento {momento}"
            )));
        }
        let mut v = Map::new();
        v.insert("resultado".into(), p.resultado.clone().into());
        v.insert("evidencia".into(), p.evidencia);
        if let Some(d) = p.detalhe.filter(|d| !d.is_empty()) {
            v.insert("detalhe".into(), d.into());
        }
        let v = Value::Object(v);
        if let Some(why) = erro_do_veredito(&v, cab, id, momento) {
            return Err(Erro::recusa(format!("veredito recusado: {why}")));
        }
        let ef = p
            .efeito
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| p.resultado.clone());
        if Resultado::parse(&ef).is_none() {
            return Err(Erro::recusa("efeito tem de ser ok, aviso ou barra"));
        }
        let modo = modos(&cfg)?.de(g.cabeca);
        let mut rec = Map::new();
        rec.insert("ts".into(), now_iso(now).into());
        rec.insert("avaliacao".into(), id_aleatorio()?.into());
        rec.insert("cabeca".into(), cab.into());
        rec.insert("guarda".into(), id.into());
        rec.insert("momento".into(), momento.into());
        if let Value::Object(v) = v {
            rec.extend(v);
        }
        rec.insert("modo".into(), modo.as_str().into());
        rec.insert("efeito".into(), ef.into());
        rec.insert("origem".into(), "emitida".into());
        rec.insert("ctx".into(), ctx_obj(&p.ctx));
        let dir = p
            .cwd
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| self.cwd_texto());
        rec.insert("dir".into(), dir.into());
        self.store()?
            .anexar_historico(std::slice::from_ref(&rec), &cfg, &self.op)?;
        Ok(rec)
    }

    /// `history` (`src/lib/cerberus.ts:552`): filtrado por cabeça e efeito (vazios não filtram), os `limite` mais
    /// novos (ausente ou zero, todos).
    pub fn historico(
        &self,
        cabeca: Option<&str>,
        efeito: Option<&str>,
        limite: Option<usize>,
    ) -> Result<Vec<Dict>, Erro> {
        let filtro = |r: &Dict, k: &str, want: Option<&str>| match want.filter(|w| !w.is_empty()) {
            None => true,
            Some(w) => r.get(k).and_then(Value::as_str) == Some(w),
        };
        let rows: Vec<Dict> = self
            .store()?
            .ler_historico()
            .into_iter()
            .filter(|r| filtro(r, "cabeca", cabeca) && filtro(r, "efeito", efeito))
            .collect();
        Ok(match limite.filter(|l| *l > 0) {
            Some(l) => rows[rows.len().saturating_sub(l)..].to_vec(),
            None => rows,
        })
    }

    /// `loadExceptions` (`src/lib/cerberus.ts:424`).
    pub fn excecoes(&self) -> Result<Vec<Excecao>, Erro> {
        self.store()?.carregar_excecoes()
    }

    /// `excecao abrir` (`src/commands/cerberus.ts:122-128`): o alvo é validado antes da validade do catálogo, e o
    /// resto em `openException` (`src/lib/cerberus.ts:451`). Só o dono abre.
    pub fn abrir_excecao(&self, p: PedidoExcecao) -> Result<Excecao, Erro> {
        let cfg = self.config(true);
        let alvo = parse_ctx(&p.alvo)?;
        let ttl_s = match p.ttl_s {
            Some(t) => t,
            None => cfg_int(&cfg, "HADES_CERBERUS_EXC_TTL_H", 1, 720)? * 3600,
        };
        let mut nova = NovaExcecao::nova(p.origem, ttl_s);
        nova.alvo = alvo.into_iter().collect();
        nova.por = p.por;
        nova.now = p.now;
        self.store()?.abrir_excecao(
            &self.registro(),
            &p.cabeca,
            &p.guarda,
            &p.momento,
            &p.motivo,
            nova,
        )
    }

    /// `state` (`src/lib/cerberus.ts:558`): o que a página Cerberus do painel mostra. `com_env` falso lê o catálogo
    /// só dos arquivos, como o painel.
    pub fn estado(&self, com_env: bool) -> Result<Dict, Erro> {
        let cfg = self.config(com_env);
        let reg = self.registro();
        let mode = modos(&cfg)?;
        let store = self.store()?;
        let hist = store.ler_historico();
        let mut barred: IndexMap<String, (Option<Value>, Option<Value>, u64, Dict)> =
            IndexMap::new();
        for r in &hist {
            if r.get("efeito").and_then(Value::as_str) != Some("barra") {
                continue;
            }
            let key = format!(
                "{}/{}",
                js_string(r.get("cabeca")),
                js_string(r.get("guarda"))
            );
            let entry = barred.entry(key).or_insert_with(|| {
                (
                    r.get("cabeca").cloned(),
                    r.get("guarda").cloned(),
                    0,
                    r.clone(),
                )
            });
            entry.2 += 1;
            entry.3 = r.clone();
        }
        let mut barrados: Vec<_> = barred.into_values().collect();
        barrados.sort_by_key(|b| std::cmp::Reverse(b.2));
        let barrados: Vec<Value> = barrados
            .into_iter()
            .map(|(cab, guarda, vezes, ultimo)| {
                let mut m = Map::new();
                if let Some(c) = cab {
                    m.insert("cabeca".into(), c);
                }
                if let Some(g) = guarda {
                    m.insert("guarda".into(), g);
                }
                m.insert("vezes".into(), vezes.into());
                m.insert("ultimo".into(), Value::Object(ultimo));
                Value::Object(m)
            })
            .collect();
        let cabecas: Vec<Value> = Cabeca::TODOS
            .iter()
            .map(|c| {
                json!({
                    "id": c.as_str(),
                    "nome": c.nome(),
                    "modo": mode.de(*c).as_str(),
                    "guardas": reg.guardas.iter().filter(|g| g.cabeca == *c).count(),
                })
            })
            .collect();
        const CAMPOS: [&str; 12] = [
            "id", "cabeca", "guarda", "momento", "alvo", "motivo", "por", "origem", "aberta",
            "expira", "usada", "usadaEm",
        ];
        let excs = store.carregar_excecoes()?;
        let excecoes: Vec<Value> = excs[excs.len().saturating_sub(EXC_LIST_MAX)..]
            .iter()
            .map(|e| {
                Value::Object(
                    CAMPOS
                        .iter()
                        .map(|k| {
                            let v = e.campos().and_then(|m| m.get(*k)).cloned();
                            ((*k).to_owned(), v.unwrap_or(Value::Null))
                        })
                        .collect(),
                )
            })
            .collect();
        let total = hist.len();
        let historico: Vec<Value> = hist[total.saturating_sub(PAGE_MAX)..]
            .iter()
            .cloned()
            .map(Value::Object)
            .collect();
        let mut out = Map::new();
        out.insert("cabecas".into(), Value::Array(cabecas));
        out.insert(
            "guardas".into(),
            serde_json::to_value(&reg.guardas).unwrap_or(Value::Null),
        );
        out.insert("erros".into(), json!(reg.erros));
        out.insert(
            "momentos".into(),
            json!(
                Momento::TODOS
                    .iter()
                    .map(|m| m.as_str())
                    .collect::<Vec<_>>()
            ),
        );
        out.insert("historico".into(), Value::Array(historico));
        out.insert("total".into(), total.into());
        out.insert("barrados".into(), Value::Array(barrados));
        out.insert("excecoes".into(), Value::Array(excecoes));
        out.insert(
            "store".into(),
            store.dir().to_string_lossy().into_owned().into(),
        );
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
