//! View-model do app: linhas, tons e contagens das telas, sem janela nem I/O.

use cerberus_core::modelo::{Cabeca, Dict, Modo, now_iso};
use cerberus_core::registro::Guarda;
use hades_ui::Tone;
use hades_ui::icons::IconName;
use serde_json::Value;

mod acoes;
pub use acoes::*;

/// Telas do trilho (`design/cerberus.html`); "Testar guarda" é o "Ensaio" do HTML (decisão 5 da F4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tela {
    Vereditos,
    Guardas,
    Modos,
    Excecoes,
    Testar,
    Configuracoes,
}

impl Tela {
    pub const TODAS: [Tela; 6] = [
        Tela::Vereditos,
        Tela::Guardas,
        Tela::Modos,
        Tela::Excecoes,
        Tela::Testar,
        Tela::Configuracoes,
    ];

    pub fn rotulo(self) -> &'static str {
        match self {
            Tela::Vereditos => "Vereditos",
            Tela::Guardas => "Guardas",
            Tela::Modos => "Modos",
            Tela::Excecoes => "Exceções",
            Tela::Testar => "Testar guarda",
            Tela::Configuracoes => "Configurações",
        }
    }

    pub fn icone(self) -> IconName {
        match self {
            Tela::Vereditos => IconName::Scale,
            Tela::Guardas => IconName::Guarda,
            Tela::Modos => IconName::ToggleRight,
            Tela::Excecoes => IconName::ShieldOff,
            Tela::Testar => IconName::Play,
            Tela::Configuracoes => IconName::SlidersHorizontal,
        }
    }

    /// Nome de tela aceito em `--tela`.
    pub fn de_nome(nome: &str) -> Option<Tela> {
        match nome {
            "vereditos" => Some(Tela::Vereditos),
            "guardas" => Some(Tela::Guardas),
            "modos" => Some(Tela::Modos),
            "excecoes" => Some(Tela::Excecoes),
            "testar" => Some(Tela::Testar),
            "configuracoes" => Some(Tela::Configuracoes),
            _ => None,
        }
    }

    pub fn de_rotulo(rotulo: &str) -> Option<Tela> {
        Tela::TODAS.into_iter().find(|t| t.rotulo() == rotulo)
    }
}

/// Atalho de teclado: `1` a `6` trocam de tela, na ordem do trilho.
pub fn tela_da_tecla(tecla: &str) -> Option<Tela> {
    let n: usize = tecla.parse().ok()?;
    Tela::TODAS.get(n.checked_sub(1)?).copied()
}

/// Controle de uma tarefa de fundo: no máximo uma em voo e resultado de geração velha é descartado (`invalidar`
/// quando a entrada mudou).
#[derive(Debug, Default)]
pub struct Tarefa {
    geracao: u64,
    em_voo: bool,
}

impl Tarefa {
    /// Geração da nova execução, ou `None` se a anterior ainda não terminou.
    pub fn iniciar(&mut self) -> Option<u64> {
        if self.em_voo {
            return None;
        }
        self.em_voo = true;
        Some(self.geracao)
    }

    /// Descarta o que estiver em voo e libera uma nova execução.
    pub fn invalidar(&mut self) {
        self.geracao = self.geracao.wrapping_add(1);
        self.em_voo = false;
    }

    /// `true` se o resultado da geração `geracao` ainda vale; só então libera a próxima execução.
    pub fn concluir(&mut self, geracao: u64) -> bool {
        if geracao != self.geracao {
            return false;
        }
        self.em_voo = false;
        true
    }

    pub fn carregando(&self) -> bool {
        self.em_voo
    }
}

/// As três cabeças, sempre nesta ordem.
pub const CABECAS: [Cabeca; 3] = [Cabeca::Porta, Cabeca::Limiar, Cabeca::Vigilia];

fn nome_da_cabeca(texto: &str) -> String {
    Cabeca::parse(texto).map_or_else(|| texto.to_owned(), |c| c.nome().to_owned())
}

pub fn texto<'a>(r: &'a Dict, chave: &str) -> &'a str {
    r.get(chave).and_then(Value::as_str).unwrap_or("")
}

/// Filtro da tela Vereditos.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filtro {
    #[default]
    Todos,
    Barrados,
    Avisos,
}

impl Filtro {
    pub const TODOS: [Filtro; 3] = [Filtro::Todos, Filtro::Barrados, Filtro::Avisos];

    pub fn rotulo(self) -> &'static str {
        match self {
            Filtro::Todos => "Todos",
            Filtro::Barrados => "Barrados",
            Filtro::Avisos => "Avisos",
        }
    }

    pub fn indice(self) -> usize {
        Filtro::TODOS.iter().position(|f| *f == self).unwrap_or(0)
    }

    pub fn de_indice(i: usize) -> Filtro {
        Filtro::TODOS.get(i).copied().unwrap_or_default()
    }

    /// O filtro olha o efeito gravado (o resultado depois do modo e das exceções).
    pub fn aceita(self, registro: &Dict) -> bool {
        match self {
            Filtro::Todos => true,
            Filtro::Barrados => texto(registro, "efeito") == "barra",
            Filtro::Avisos => texto(registro, "efeito") == "aviso",
        }
    }
}

/// Rótulo e tom do efeito: barrado em vermelho, aviso em âmbar, passa em verde.
pub fn veredito(efeito: &str) -> (&'static str, Tone) {
    match efeito {
        "barra" => ("barrado", Tone::Bad),
        "aviso" => ("aviso", Tone::Warn),
        "ok" => ("passa", Tone::Ok),
        "liberado" => ("liberado", Tone::Info),
        _ => ("—", Tone::Neutral),
    }
}

/// `2026-11-21T13:02:44.000Z` → `13:02`; texto que não é ISO volta como veio.
pub fn hora(ts: &str) -> String {
    match ts.get(11..16) {
        Some(h) if ts.as_bytes().get(10) == Some(&b'T') => h.to_owned(),
        _ => ts.to_owned(),
    }
}

/// `2026-11-21T13:02:44.000Z` → `2026-11-21 13:02:44`.
pub fn data_hora(ts: &str) -> String {
    match (ts.get(..10), ts.get(11..19)) {
        (Some(dia), Some(h)) if ts.as_bytes().get(10) == Some(&b'T') => format!("{dia} {h}"),
        _ => ts.to_owned(),
    }
}

const MASCARA: &str = "●●●●●●●●";
const TOKEN_MIN: usize = 16;

fn parece_segredo(palavra: &str) -> bool {
    let n = palavra.chars().count();
    n >= TOKEN_MIN
        && palavra
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '/' | '.'))
        && palavra.chars().any(|c| c.is_ascii_digit())
        && palavra.chars().any(|c| c.is_ascii_alphabetic())
        && !palavra.contains('/')
}

/// Texto da evidência sem valor: o que vem depois de `=` em `CHAVE=valor` e palavra com cara de token (16+
/// caracteres de letras e dígitos) viram `●●●●●●●●`. Arquivo e linha ficam.
pub fn mascarar(texto: &str) -> String {
    texto
        .split(' ')
        .map(|palavra| {
            if let Some((chave, valor)) = palavra.split_once('=')
                && !chave.is_empty()
                && !valor.is_empty()
            {
                return format!("{chave}={MASCARA}");
            }
            if parece_segredo(palavra) {
                return MASCARA.to_owned();
            }
            palavra.to_owned()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Um item da evidência como o inspector mostra: onde (arquivo e linha) e o que foi conferido, mascarado. Os outros
/// campos que a guarda gravar (trecho, valor) nunca saem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemEvidencia {
    pub onde: String,
    pub conferido: String,
}

pub fn evidencia(registro: &Dict) -> Vec<ItemEvidencia> {
    registro
        .get("evidencia")
        .and_then(Value::as_array)
        .map(|itens| {
            itens
                .iter()
                .filter_map(Value::as_object)
                .map(|o| ItemEvidencia {
                    onde: mascarar(texto(o, "onde")),
                    conferido: mascarar(texto(o, "conferido")),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Uma linha da tabela de vereditos.
#[derive(Clone, Debug, PartialEq)]
pub struct LinhaVeredito {
    pub hora: String,
    pub cabeca: String,
    pub guarda: String,
    pub momento: String,
    pub veredito: &'static str,
    pub tom: Tone,
    pub evidencia: String,
}

impl LinhaVeredito {
    pub fn celulas(&self) -> [String; 6] {
        [
            self.hora.clone(),
            self.cabeca.clone(),
            self.guarda.clone(),
            self.momento.clone(),
            self.veredito.to_owned(),
            self.evidencia.clone(),
        ]
    }
}

pub const COLUNAS_VEREDITOS: [&str; 6] = [
    "Hora",
    "Cabeça",
    "Guarda",
    "Momento",
    "Veredito",
    "Evidência",
];

fn resumo_da_evidencia(registro: &Dict) -> String {
    match evidencia(registro).first() {
        Some(e) if e.onde.is_empty() => e.conferido.clone(),
        Some(e) => e.onde.clone(),
        None => String::new(),
    }
}

pub fn linha_veredito(registro: &Dict) -> LinhaVeredito {
    let (veredito, tom) = veredito(texto(registro, "efeito"));
    LinhaVeredito {
        hora: hora(texto(registro, "ts")),
        cabeca: nome_da_cabeca(texto(registro, "cabeca")),
        guarda: texto(registro, "guarda").to_owned(),
        momento: texto(registro, "momento").to_owned(),
        veredito,
        tom,
        evidencia: resumo_da_evidencia(registro),
    }
}

/// Índices do histórico que o filtro mostra, do mais novo para o mais velho.
pub fn visiveis(historico: &[Dict], filtro: Filtro) -> Vec<usize> {
    (0..historico.len())
        .rev()
        .filter(|i| filtro.aceita(&historico[*i]))
        .collect()
}

/// Chave estável de um veredito para manter a seleção entre recargas.
pub fn chave(registro: &Dict) -> String {
    format!(
        "{}|{}|{}|{}",
        texto(registro, "ts"),
        texto(registro, "avaliacao"),
        texto(registro, "cabeca"),
        texto(registro, "guarda")
    )
}

/// Posição na lista visível do veredito selecionado; sem ele (ou fora do filtro), nenhuma.
pub fn posicao_da_selecao(
    historico: &[Dict],
    visiveis: &[usize],
    selecionado: Option<&str>,
) -> Option<usize> {
    let alvo = selecionado?;
    visiveis.iter().position(|i| chave(&historico[*i]) == alvo)
}

/// Indicadores de uma cabeça nas últimas 24 h.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KpiCabeca {
    pub cabeca: Cabeca,
    pub vereditos: usize,
    pub barrados: usize,
}

pub const JANELA_S: f64 = 24.0 * 3600.0;

/// Vereditos e barrados por cabeça desde `agora - 24 h`. O `ts` é ISO UTC de largura fixa, então a comparação de
/// texto basta.
pub fn kpis(historico: &[Dict], agora_s: f64) -> Vec<KpiCabeca> {
    let desde = now_iso(agora_s - JANELA_S);
    CABECAS
        .iter()
        .map(|cab| {
            let da_cabeca = historico
                .iter()
                .filter(|r| texto(r, "cabeca") == cab.as_str() && texto(r, "ts") >= desde.as_str());
            let (mut vereditos, mut barrados) = (0, 0);
            for r in da_cabeca {
                vereditos += 1;
                if texto(r, "efeito") == "barra" {
                    barrados += 1;
                }
            }
            KpiCabeca {
                cabeca: *cab,
                vereditos,
                barrados,
            }
        })
        .collect()
}

/// O que o inspector mostra do veredito selecionado.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspecao {
    pub titulo: String,
    pub veredito: &'static str,
    pub tom: Tone,
    pub meta: Vec<(&'static str, String)>,
    pub evidencia: Vec<ItemEvidencia>,
    pub detalhe: Option<String>,
}

pub fn inspecao(registro: &Dict) -> Inspecao {
    let (veredito, tom) = veredito(texto(registro, "efeito"));
    let mut meta = vec![
        ("Quando (UTC)", data_hora(texto(registro, "ts"))),
        ("Cabeça", nome_da_cabeca(texto(registro, "cabeca"))),
        ("Momento", texto(registro, "momento").to_owned()),
        ("Resultado", texto(registro, "resultado").to_owned()),
        ("Modo", texto(registro, "modo").to_owned()),
    ];
    if let Some(id) = registro
        .get("excecao")
        .and_then(Value::as_object)
        .map(|e| texto(e, "id"))
        .filter(|id| !id.is_empty())
    {
        meta.push(("Exceção", id.to_owned()));
    }
    let detalhe = registro
        .get("detalhe")
        .and_then(Value::as_str)
        .filter(|d| !d.is_empty())
        .map(mascarar);
    Inspecao {
        titulo: texto(registro, "guarda").to_owned(),
        veredito,
        tom,
        meta,
        evidencia: evidencia(registro),
        detalhe,
    }
}

/// Uma linha da tela Guardas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinhaGuarda {
    pub id: String,
    pub tipo: String,
    pub momentos: String,
    pub cmd: String,
}

pub const COLUNAS_GUARDAS: [&str; 4] = ["Id", "Tipo", "Momentos", "Comando"];

/// As guardas de uma cabeça, na ordem do registro.
pub fn guardas_da_cabeca(guardas: &[Guarda], cab: Cabeca) -> Vec<LinhaGuarda> {
    guardas
        .iter()
        .filter(|g| g.cabeca == cab)
        .map(|g| LinhaGuarda {
            id: g.id.clone(),
            tipo: g.tipo.as_str().to_owned(),
            momentos: g
                .momentos
                .iter()
                .map(|m| m.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            cmd: if g.cmd.is_empty() {
                "—".to_owned()
            } else {
                g.cmd.join(" ")
            },
        })
        .collect()
}

/// Os erros do `--validar` (`<cabeça>/<arquivo>: …`) agrupados por cabeça, na ordem Porta, Limiar, Vigília; o que
/// não começa por uma cabeça vai por último, em "Registro".
pub fn erros_por_cabeca(erros: &[String]) -> Vec<(String, Vec<String>)> {
    let mut grupos: Vec<(usize, String, Vec<String>)> = Vec::new();
    for e in erros {
        let ordem = CABECAS
            .iter()
            .position(|c| e.starts_with(&format!("{}/", c.as_str())))
            .unwrap_or(CABECAS.len());
        let nome = CABECAS
            .get(ordem)
            .map_or_else(|| "Registro".to_owned(), |c| c.nome().to_owned());
        match grupos.iter_mut().find(|g| g.0 == ordem) {
            Some(g) => g.2.push(e.clone()),
            None => grupos.push((ordem, nome, vec![e.clone()])),
        }
    }
    grupos.sort_by_key(|g| g.0);
    grupos.into_iter().map(|g| (g.1, g.2)).collect()
}

/// De onde vem o modo efetivo de uma cabeça.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origem {
    Env,
    ConfigEnv,
    Padrao,
}

impl Origem {
    pub fn rotulo(self) -> &'static str {
        match self {
            Origem::Env => "variável de ambiente",
            Origem::ConfigEnv => "config.env",
            Origem::Padrao => "padrão",
        }
    }
}

/// Chave de configuração do modo de uma cabeça (`HADES_CERBERUS_PORTA`).
pub fn chave_do_modo(cab: Cabeca) -> String {
    format!("HADES_CERBERUS_{}", cab.as_str().to_uppercase())
}

/// A mesma precedência do `config` do core: variável exportada (vazia inclusive) > linha do `config.env` > padrão.
pub fn origem_do_modo(chave: &str, no_env: bool, config_env: Option<&str>) -> Origem {
    if no_env {
        return Origem::Env;
    }
    let no_arquivo = config_env.is_some_and(|t| {
        t.lines()
            .any(|l| l.split_once('=').is_some_and(|(k, _)| k == chave))
    });
    if no_arquivo {
        Origem::ConfigEnv
    } else {
        Origem::Padrao
    }
}

/// Rótulo e tom do modo: barra em vermelho, avisa em âmbar, desligado neutro.
pub fn rotulo_do_modo(modo: Modo) -> (&'static str, Tone) {
    match modo {
        Modo::Barra => ("barra", Tone::Bad),
        Modo::Avisa => ("avisa", Tone::Warn),
        _ => ("desligado", Tone::Neutral),
    }
}

pub fn explicacao_do_modo(modo: Modo) -> &'static str {
    match modo {
        Modo::Barra => "Veredito barra bloqueia o fluxo.",
        Modo::Avisa => "Veredito barra vira aviso: registra e deixa passar.",
        _ => "A cabeça não avalia.",
    }
}

/// O que a tela Modos mostra de uma cabeça.
#[derive(Clone, Debug, PartialEq)]
pub struct CartaoModo {
    pub cabeca: Cabeca,
    pub chave: String,
    pub modo: Modo,
    pub origem: Origem,
}

fn plural(n: usize, singular: &str, plural: &str) -> String {
    format!("{n} {}", if n == 1 { singular } else { plural })
}

pub fn rodape_dos_vereditos(total: usize) -> String {
    plural(total, "veredito", "vereditos")
}

pub fn rodape_das_guardas(total: usize, erros: usize) -> String {
    let base = plural(total, "guarda", "guardas");
    if erros == 0 {
        base
    } else {
        format!("{base} · {}", plural(erros, "erro", "erros"))
    }
}

/// Modo de um texto de configuração (aparado), ou `None` se não for um dos três.
pub fn modo_do_texto(texto: &str) -> Option<Modo> {
    Modo::parse(texto.trim())
}

/// Nome da cabeça dona da chave de modo, ou a própria chave.
pub fn nome_da_chave_de_modo(chave: &str) -> String {
    CABECAS
        .iter()
        .find(|c| chave_do_modo(**c) == chave)
        .map_or_else(|| chave.to_owned(), |c| c.nome().to_owned())
}

pub fn rotulo_da_origem(origem: crate::config::Origem) -> &'static str {
    match origem {
        crate::config::Origem::Ambiente => Origem::Env.rotulo(),
        crate::config::Origem::Arquivo => Origem::ConfigEnv.rotulo(),
        crate::config::Origem::Padrao => Origem::Padrao.rotulo(),
    }
}

pub fn configuracoes(n: usize) -> String {
    plural(n, "configuração", "configurações")
}

/// Aviso do rodapé depois de gravar.
pub fn gravadas(n: usize) -> String {
    match n {
        0 => "Nada a gravar".to_owned(),
        _ => format!("Configurações gravadas ({n})"),
    }
}

#[cfg(test)]
mod tests;
