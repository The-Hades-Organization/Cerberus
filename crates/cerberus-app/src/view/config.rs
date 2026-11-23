//! Tela Configurações: os modos das cabeças em destaque e as demais chaves `HADES_CERBERUS_*` em grupos.

use std::sync::Arc;

use gpui::{AppContext, Context, Entity, ParentElement, SharedString, Styled, Window, div, px};
use gpui_component::input::{InputEvent, InputState};
use hades_ui::components::{
    Card, EmptyState, MetaList, Page, PageHeader, Segmented, SettingOrigin, SettingRow,
    SettingsBar, SettingsGroup, StatusBadge,
};
use hades_ui::tokens::space;

use super::CerberusView;
use crate::config::{self, FonteDeConfig, Linha, Origem};
use crate::vm;

/// Campos da página, criados quando a leitura chega e a janela está à mão.
pub(super) struct CfgEstado {
    pub linhas: Vec<Linha>,
    pub entradas: Vec<Entity<InputState>>,
}

/// O que a leitura de fundo trouxe e ainda não virou campo.
pub(super) type LeituraCfg = Result<Vec<Linha>, String>;

fn origem(o: Origem) -> SettingOrigin {
    match o {
        Origem::Padrao => SettingOrigin::Default,
        Origem::Arquivo => SettingOrigin::File,
        Origem::Ambiente => SettingOrigin::Env,
    }
}

impl CerberusView {
    /// Lê o catálogo no executor de fundo; leitura velha é descartada.
    pub(super) fn carregar_config(&mut self, cx: &mut Context<Self>) {
        let Some(g) = self.t_cfg.iniciar() else {
            return;
        };
        let fonte = self.fonte.clone();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { fonte.carregar() })
                .await;
            let _ = this.update(cx, |v, cx| {
                if !v.t_cfg.concluir(g) {
                    return;
                }
                v.cfg_lida = Some(r);
                v.cfg = None;
                cx.notify();
            });
        })
        .detach();
    }

    /// Troca a leitura recém-chegada pelos campos; precisa da janela para criar os `InputState`.
    pub(super) fn garantir_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Ok(linhas)) = self.cfg_lida.take_if(|r| r.is_ok()) else {
            return;
        };
        let mut entradas = Vec::new();
        for l in &linhas {
            let valor = l.valor.clone();
            let entrada = cx.new(|cx| InputState::new(window, cx).default_value(valor));
            cx.subscribe(&entrada, |v: &mut Self, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    v.cfg_salvo = false;
                    cx.notify();
                }
            })
            .detach();
            entradas.push(entrada);
        }
        self.cfg = Some(CfgEstado { linhas, entradas });
    }

    fn textos_config(&self, cx: &gpui::App) -> Vec<String> {
        self.cfg.as_ref().map_or_else(Vec::new, |c| {
            c.entradas
                .iter()
                .map(|e| e.read(cx).value().to_string())
                .collect()
        })
    }

    /// Grava no fundo; ao terminar relê o catálogo e os dados das outras telas (os modos mudam).
    fn salvar_config(&mut self, cx: &mut Context<Self>) {
        let textos = self.textos_config(cx);
        let Some(c) = &self.cfg else { return };
        let Some(g) = self.t_salvar.iniciar() else {
            return;
        };
        let linhas = c.linhas.clone();
        let fonte: Arc<dyn FonteDeConfig> = self.fonte.clone();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { config::salvar(fonte.as_ref(), &linhas, &textos) })
                .await;
            let _ = this.update(cx, |v, cx| {
                if !v.t_salvar.concluir(g) {
                    return;
                }
                match r {
                    Ok(n) => {
                        v.aviso = Some(vm::gravadas(n));
                        v.cfg_salvo = true;
                        v.t_cfg.invalidar();
                        v.carregar_config(cx);
                        v.forcar_recarga(cx);
                    }
                    Err(e) => v.aviso = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn descartar_config(&mut self, cx: &mut Context<Self>) {
        self.cfg_salvo = false;
        self.t_cfg.invalidar();
        self.carregar_config(cx);
        cx.notify();
    }

    fn cartao_de_modo(&self, i: usize, linha: &Linha, texto: &str) -> Card {
        let opcoes = linha.opcoes().unwrap_or_default();
        let atual = opcoes.iter().position(|o| o == texto.trim());
        let (rotulo, tom) = match vm::modo_do_texto(texto) {
            Some(m) => vm::rotulo_do_modo(m),
            None => ("inválido", hades_ui::Tone::Bad),
        };
        let mut cartao = Card::new()
            .title(vm::nome_da_chave_de_modo(&linha.chave))
            .status(StatusBadge::new(rotulo, tom))
            .child(
                MetaList::new()
                    .label_width(72.0)
                    .mono("Chave", linha.chave.clone())
                    .row("Origem", vm::rotulo_da_origem(linha.origem)),
            );
        if linha.somente_leitura() {
            return cartao
                .note("Definido no ambiente: só muda fora do app.")
                .fill();
        }
        let entrada = self.cfg.as_ref().map(|c| c.entradas[i].clone());
        let mut seg = Segmented::new(
            SharedString::from(format!("modo-{}", linha.chave)),
            opcoes.clone(),
        );
        if let Some(p) = atual {
            seg = seg.selected(p);
        }
        let seg = seg.on_select(move |p, window, cx| {
            if let (Some(e), Some(o)) = (&entrada, opcoes.get(p)) {
                let o = o.clone();
                e.update(cx, |s, cx| s.set_value(o, window, cx));
            }
        });
        cartao = cartao.child(seg);
        cartao.fill()
    }

    pub(super) fn tela_config(&self, cx: &mut Context<Self>) -> Page {
        let cabecalho = PageHeader::new("Configurações")
            .subtitle("Chaves HADES_CERBERUS_* do catálogo do Hades")
            .description("Mesmos nomes, padrões, faixas e precedência do CLI: ambiente, depois o config.env, depois o padrão.");
        let pagina = Page::new("pagina-config").header(cabecalho);
        if let Some(Err(erro)) = &self.cfg_lida {
            return pagina.child(
                EmptyState::new("Catálogo indisponível")
                    .hint(erro.clone())
                    .fill(),
            );
        }
        let Some(c) = &self.cfg else {
            return pagina.child(self.esqueleto("config"));
        };
        let textos = self.textos_config(cx);
        let erros = config::erros(&c.linhas, &textos);
        let pendentes = config::pendentes(&c.linhas, &textos);
        let mut pagina = pagina;
        if !self.fonte.gravavel() {
            pagina = pagina.child(Card::new().note(
                "Sem instalação do Hades: a página mostra os padrões do kit e não grava nada.",
            ));
        }
        let (modos, outras): (Vec<usize>, Vec<usize>) =
            (0..c.linhas.len()).partition(|&i| c.linhas[i].opcoes().is_some());
        if !modos.is_empty() {
            let mut linha = div().flex().gap(px(space::MD)).min_w_0();
            for i in modos {
                let cartao = self.cartao_de_modo(i, &c.linhas[i], &textos[i]);
                linha = linha.child(div().flex().flex_col().flex_1().child(cartao));
            }
            pagina = pagina.child(linha);
        }
        let mut grupos: Vec<(String, Vec<usize>)> = Vec::new();
        for i in outras {
            let g = &c.linhas[i].grupo;
            match grupos.iter_mut().find(|(n, _)| n == g) {
                Some((_, idx)) => idx.push(i),
                None => grupos.push((g.clone(), vec![i])),
            }
        }
        for (nome, indices) in grupos {
            let fora = indices
                .iter()
                .filter(|&&i| textos[i].trim() != c.linhas[i].padrao)
                .count();
            let mut grupo = SettingsGroup::new(nome).meta(format!(
                "{} · {fora} fora do padrão",
                vm::configuracoes(indices.len())
            ));
            for i in indices {
                let l = &c.linhas[i];
                let id = SharedString::from(format!("cfg-{}", l.chave));
                let entrada = &c.entradas[i];
                let mut linha = match l.faixa() {
                    Some((min, max)) => SettingRow::number(id, l.rotulo.clone(), entrada, min, max),
                    None => SettingRow::text(id, l.rotulo.clone(), entrada),
                }
                .key(l.chave.clone())
                .description(l.descricao.clone())
                .default_value(l.padrao.clone())
                .origin(origem(l.origem))
                .modified(textos[i].trim() != l.padrao)
                .disabled(l.somente_leitura())
                .error_opt(erros[i].clone());
                let (entrada, padrao) = (entrada.clone(), l.padrao.clone());
                linha = linha.on_reset(move |_, window, cx| {
                    entrada.update(cx, |s, cx| s.set_value(padrao.clone(), window, cx));
                });
                grupo = grupo.row(linha);
            }
            pagina = pagina.child(grupo);
        }
        let entidade = cx.entity();
        let (e_salvar, e_descartar) = (entidade.clone(), entidade);
        pagina.child(
            SettingsBar::new(pendentes)
                .saving(self.t_salvar.carregando())
                .saved(self.cfg_salvo)
                .on_save(move |_, _, cx| e_salvar.update(cx, |v, cx| v.salvar_config(cx)))
                .on_discard(move |_, _, cx| e_descartar.update(cx, |v, cx| v.descartar_config(cx))),
        )
    }
}
