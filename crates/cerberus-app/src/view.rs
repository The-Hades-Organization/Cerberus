//! `CerberusView`: casca (rail, status) e as telas Vereditos, Guardas e Modos; as de ação ficam nos submódulos.

mod config;
mod excecoes;
mod tabela;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, px,
};
use hades_core::Env;
use hades_ui::components::{
    Align, Button, Card, DataTable, EmptyState, Kpi, KpiStrip, MetaList, Page, PageHeader, Rail,
    RailBrand, RailGroup, RailItem, Segmented, Skeleton, StatusBadge, StatusBar, TermBlock,
};
use hades_ui::icons::IconName;
use hades_ui::tokens::{FONT_HEADING, size, space};
use hades_ui::{Tone, palette};

use crate::config::{ConfigLocal, FonteDeConfig};
use crate::service::{self, Dados};
use tabela::TabelaTonada;

use crate::vm::{self, CABECAS, COLUNAS_GUARDAS, COLUNAS_VEREDITOS, Filtro, Tarefa, Tela};

const PERIODO_MS: u64 = 1000;
const TIQUES_DADOS: u32 = 5;
const LARGURA_INSPECTOR: f32 = 360.0;

pub struct CerberusView {
    env: Env,
    cwd: PathBuf,
    tela: Tela,
    dados: Option<Dados>,
    aviso: Option<String>,
    filtro: Filtro,
    selecionado: Option<String>,
    foco: FocusHandle,
    tique: u32,
    t_dados: Tarefa,
    recolhida: bool,
    fonte: Arc<dyn FonteDeConfig>,
    cfg: Option<config::CfgEstado>,
    cfg_lida: Option<config::LeituraCfg>,
    cfg_salvo: bool,
    t_cfg: Tarefa,
    t_salvar: Tarefa,
    form: excecoes::FormExcecao,
    ensaio: excecoes::Ensaio,
}

impl CerberusView {
    pub fn new(env: Env, cwd: PathBuf, foco: FocusHandle, cx: &mut Context<Self>) -> Self {
        let fonte: Arc<dyn FonteDeConfig> = Arc::new(ConfigLocal::new(&env, &cwd));
        let mut view = Self {
            env,
            cwd,
            tela: Tela::Vereditos,
            dados: None,
            aviso: None,
            filtro: Filtro::default(),
            selecionado: None,
            foco,
            tique: 0,
            t_dados: Tarefa::default(),
            recolhida: false,
            fonte,
            cfg: None,
            cfg_lida: None,
            cfg_salvo: false,
            t_cfg: Tarefa::default(),
            t_salvar: Tarefa::default(),
            form: excecoes::FormExcecao::default(),
            ensaio: excecoes::Ensaio::default(),
        };
        view.recarregar(cx);
        view
    }

    /// Lê histórico, registro e modos no executor de fundo; resultado de uma geração velha é descartado.
    fn recarregar(&mut self, cx: &mut Context<Self>) {
        let Some(g) = self.t_dados.iniciar() else {
            return;
        };
        let opcoes = service::opcoes(self.env.clone(), &self.cwd);
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { service::carregar(opcoes) })
                .await;
            let _ = this.update(cx, |v, cx| {
                if !v.t_dados.concluir(g) {
                    return;
                }
                match r {
                    Ok(d) => {
                        v.dados = Some(d);
                        v.aviso = None;
                    }
                    Err(e) => v.aviso = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn forcar_recarga(&mut self, cx: &mut Context<Self>) {
        self.t_dados.invalidar();
        self.recarregar(cx);
    }

    fn tiquetaque(&mut self, cx: &mut Context<Self>) {
        self.tique = self.tique.wrapping_add(1);
        if self.tique.is_multiple_of(TIQUES_DADOS) {
            self.recarregar(cx);
        }
    }

    pub fn iniciar(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(PERIODO_MS))
                    .await;
                if this.update(cx, |v, cx| v.tiquetaque(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn mudar_tela(&mut self, tela: Tela, cx: &mut Context<Self>) {
        self.tela = tela;
        if tela == Tela::Configuracoes && self.cfg.is_none() {
            self.carregar_config(cx);
        }
        self.recarregar(cx);
    }

    fn historico(&self) -> &[cerberus_core::modelo::Dict] {
        self.dados.as_ref().map_or(&[], |d| d.historico.as_slice())
    }

    fn visiveis(&self) -> Vec<usize> {
        vm::visiveis(self.historico(), self.filtro)
    }

    fn selecionar(&mut self, posicao: usize) {
        let visiveis = self.visiveis();
        if let Some(i) = visiveis.get(posicao) {
            self.selecionado = Some(vm::chave(&self.historico()[*i]));
        }
    }

    fn esqueleto(&self, id: &'static str) -> gpui::AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .child(Skeleton::new(format!("{id}-1")))
            .child(Skeleton::new(format!("{id}-2")))
            .child(Skeleton::new(format!("{id}-3")))
            .into_any_element()
    }

    fn rodape(&self) -> (String, String) {
        let esquerda = self.aviso.clone().unwrap_or_else(|| {
            if self.dados.is_none() && self.t_dados.carregando() {
                return "carregando…".to_owned();
            }
            let Some(d) = &self.dados else {
                return String::new();
            };
            match self.tela {
                Tela::Guardas => vm::rodape_das_guardas(d.guardas.len(), d.erros.len()),
                _ => vm::rodape_dos_vereditos(d.historico.len()),
            }
        });
        let direita = self
            .dados
            .as_ref()
            .and_then(|d| d.pasta.as_ref())
            .map(|p| p.join("vereditos.jsonl").display().to_string())
            .unwrap_or_default();
        (esquerda, direita)
    }

    fn rail(&self, cx: &mut Context<Self>) -> Rail {
        let entidade = cx.entity();
        let recolhimento = cx.entity();
        let item = |t: Tela| RailItem::new(t.rotulo()).icon(t.icone());
        Rail::new([
            RailGroup::new(
                "Consulta",
                [
                    item(Tela::Vereditos),
                    item(Tela::Guardas),
                    item(Tela::Modos),
                ],
            ),
            RailGroup::new("Ação", [item(Tela::Excecoes), item(Tela::Testar)]),
            RailGroup::new("Sistema", [item(Tela::Configuracoes)]),
        ])
        .brand(
            RailBrand::new("Cerberus")
                .icon(IconName::Cerberus)
                .subtitle("guardas"),
        )
        .theme_toggle()
        .collapsed(self.recolhida)
        .on_collapse(move |recolher, _, cx| {
            recolhimento.update(cx, |v, cx| {
                v.recolhida = recolher;
                cx.notify();
            });
        })
        .selected(self.tela.rotulo())
        .on_select(move |rotulo, _, cx| {
            if let Some(t) = Tela::de_rotulo(rotulo) {
                entidade.update(cx, |v, cx| {
                    v.mudar_tela(t, cx);
                    cx.notify();
                });
            }
        })
    }

    fn botao_recarregar(&self, cx: &mut Context<Self>) -> Button {
        let e = cx.entity();
        Button::new("recarregar", "Recarregar")
            .icon(IconName::RefreshCw)
            .loading(self.t_dados.carregando())
            .on_click(move |_, _, cx| e.update(cx, |v, cx| v.forcar_recarga(cx)))
    }

    fn kpis(&self, d: &Dados) -> KpiStrip {
        let mut itens = Vec::new();
        for k in vm::kpis(&d.historico, d.agora_s) {
            let nome = k.cabeca.nome();
            itens.push(Kpi::new(format!("{nome} · 24 h"), k.vereditos.to_string()));
            let tom = if k.barrados > 0 {
                Tone::Bad
            } else {
                Tone::Neutral
            };
            itens.push(Kpi::new(format!("{nome} · barrados"), k.barrados.to_string()).tone(tom));
        }
        KpiStrip::new(itens)
    }

    fn tabela_dos_vereditos(&self, visiveis: &[usize], cx: &mut Context<Self>) -> TabelaTonada {
        let entidade = cx.entity();
        let mut tabela = TabelaTonada::new(
            "tabela-vereditos",
            COLUNAS_VEREDITOS.iter().map(|n| SharedString::from(*n)),
            "Nenhum veredito neste filtro",
        );
        let hist = self.historico();
        for i in visiveis {
            let linha = vm::linha_veredito(&hist[*i]);
            tabela = tabela.linha(
                linha
                    .celulas()
                    .into_iter()
                    .map(SharedString::from)
                    .zip(linha.tons()),
            );
        }
        if let Some(p) = vm::posicao_da_selecao(hist, visiveis, self.selecionado.as_deref()) {
            tabela = tabela.selecionada(p);
        }
        tabela.ao_escolher(move |p, _, cx| {
            entidade.update(cx, |v, cx| {
                v.selecionar(p);
                cx.notify();
            });
        })
    }

    fn inspector(&self, visiveis: &[usize], cx: &mut Context<Self>) -> gpui::AnyElement {
        let hist = self.historico();
        let Some(p) = vm::posicao_da_selecao(hist, visiveis, self.selecionado.as_deref()) else {
            return EmptyState::new("Nenhum veredito selecionado")
                .hint("Escolha uma linha para ver a evidência.")
                .fill()
                .into_any_element();
        };
        let reg = &hist[visiveis[p]];
        let [cab, guarda, momento] =
            ["cabeca", "guarda", "momento"].map(|k| vm::texto(reg, k).to_owned());
        let i = vm::inspecao(reg);
        let mut meta = MetaList::new().label_width(104.0);
        for (rotulo, valor) in i.meta {
            meta = meta.row(rotulo, valor);
        }
        let mut evidencia = MetaList::new().label_width(104.0);
        if i.evidencia.is_empty() {
            evidencia = evidencia.row("Evidência", "—");
        }
        for e in &i.evidencia {
            evidencia = evidencia
                .mono("Onde", e.onde.clone())
                .row("Conferido", e.conferido.clone());
        }
        let titulo = div()
            .flex()
            .items_start()
            .gap(px(space::MD))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(FONT_HEADING)
                    .text_size(px(size::SECTION))
                    .child(i.titulo),
            )
            .child(div().flex_none().child(StatusBadge::new(i.veredito, i.tom)));
        let mut cartao = Card::new().child(titulo).child(meta).child(evidencia);
        if let Some(d) = i.detalhe {
            cartao = cartao.child(TermBlock::new().line(d));
        }
        cartao
            .child(
                div().flex().child(
                    Button::new("criar-excecao", "Criar exceção")
                        .icon(IconName::ShieldOff)
                        .on_click(cx.listener(move |v, _, _, cx| {
                            v.criar_excecao_de(&cab, &guarda, &momento, cx)
                        })),
                ),
            )
            .note("O trecho achado nunca aparece inteiro.")
            .into_any_element()
    }

    fn tela_vereditos(&self, cx: &mut Context<Self>) -> Page {
        let cabecalho = PageHeader::new("Vereditos")
            .subtitle("O que as três cabeças decidiram")
            .description("Barrado em vermelho, aviso em âmbar, passa em verde. A evidência mostra arquivo e linha, nunca o valor.")
            .action(self.botao_recarregar(cx));
        let pagina = Page::new("pagina-vereditos").header(cabecalho);
        let Some(d) = &self.dados else {
            return pagina.child(self.esqueleto("vereditos"));
        };
        let entidade = cx.entity();
        let filtro = Segmented::new("filtro", Filtro::TODOS.map(Filtro::rotulo))
            .selected(self.filtro.indice())
            .on_select(move |i, _, cx| {
                entidade.update(cx, |v, cx| {
                    v.filtro = Filtro::de_indice(i);
                    cx.notify();
                });
            });
        let visiveis = self.visiveis();
        let corpo = div()
            .flex()
            .flex_1()
            .min_h_0()
            .gap(px(space::LG))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(self.tabela_dos_vereditos(&visiveis, cx)),
            )
            .child(
                div()
                    .id("inspector")
                    .flex_none()
                    .w(px(LARGURA_INSPECTOR))
                    .overflow_y_scroll()
                    .child(self.inspector(&visiveis, cx)),
            );
        pagina
            .child(self.kpis(d))
            .child(div().flex().child(filtro))
            .fill_child(corpo)
    }

    fn tela_guardas(&self, cx: &mut Context<Self>) -> Page {
        let cabecalho = PageHeader::new("Guardas")
            .subtitle("O registro de cada cabeça")
            .description("Lido da skill a cada recarga. Arquivo inválido aparece em destaque e fica fora da avaliação.")
            .action(self.botao_recarregar(cx));
        let mut pagina = Page::new("pagina-guardas").header(cabecalho);
        let Some(d) = &self.dados else {
            return pagina.child(self.esqueleto("guardas"));
        };
        if !d.erros.is_empty() {
            let mut cartao = Card::new()
                .title("Registro inválido")
                .badge(format!("{} erros", d.erros.len()), Tone::Bad);
            for (nome, erros) in vm::erros_por_cabeca(&d.erros) {
                let mut bloco = TermBlock::new().dim(nome);
                for e in erros {
                    bloco = bloco.bad(e);
                }
                cartao = cartao.child(bloco);
            }
            pagina = pagina.child(cartao);
        }
        for cab in CABECAS {
            let linhas = vm::guardas_da_cabeca(&d.guardas, cab);
            let colunas = COLUNAS_GUARDAS
                .iter()
                .map(|n| (SharedString::from(*n), Align::Start))
                .collect::<Vec<_>>();
            let mut tabela = DataTable::new(colunas).empty("Nenhuma guarda registrada");
            for l in &linhas {
                tabela = tabela.row(
                    [
                        l.id.clone(),
                        l.tipo.clone(),
                        l.momentos.clone(),
                        l.cmd.clone(),
                    ]
                    .into_iter()
                    .map(SharedString::from),
                );
            }
            pagina = pagina.child(
                Card::new()
                    .title(cab.nome())
                    .meta(format!("{} guardas", linhas.len()))
                    .child(tabela),
            );
        }
        pagina
    }

    fn tela_modos(&self, cx: &mut Context<Self>) -> Page {
        let cabecalho = PageHeader::new("Modos")
            .subtitle("O modo efetivo de cada cabeça e de onde ele vem")
            .description("Variável de ambiente vence o config.env, que vence o padrão da skill.");
        let pagina = Page::new("pagina-modos").header(cabecalho);
        let Some(d) = &self.dados else {
            return pagina.child(self.esqueleto("modos"));
        };
        let cartoes = match &d.modos {
            Ok(c) => c,
            Err(e) => {
                return pagina.child(
                    EmptyState::new("Configuração inválida")
                        .hint(e.clone())
                        .fill(),
                );
            }
        };
        let mut linha = div().flex().gap(px(space::MD)).min_w_0();
        for c in cartoes {
            let (rotulo, tom) = vm::rotulo_do_modo(c.modo);
            let e = cx.entity();
            let cartao = Card::new()
                .title(c.cabeca.nome())
                .status(StatusBadge::new(rotulo, tom))
                .child(
                    MetaList::new()
                        .label_width(72.0)
                        .mono("Chave", c.chave.clone())
                        .row("Origem", c.origem.rotulo()),
                )
                .child(vm::explicacao_do_modo(c.modo))
                .action(
                    Button::new(
                        SharedString::from(format!("editar-{}", c.cabeca.as_str())),
                        "Editar",
                    )
                    .icon(IconName::SlidersHorizontal)
                    .on_click(move |_, _, cx| {
                        e.update(cx, |v, cx| {
                            v.mudar_tela(Tela::Configuracoes, cx);
                            cx.notify();
                        })
                    }),
                )
                .fill();
            linha = linha.child(div().flex().flex_col().flex_1().child(cartao));
        }
        pagina.child(linha)
    }
}

impl CerberusView {
    fn campo_focado(&self, window: &Window, cx: &App) -> bool {
        let cfg = self.cfg.iter().flat_map(|c| c.entradas.iter());
        let form = [
            &self.form.motivo,
            &self.form.ttl,
            &self.form.alvo,
            &self.ensaio.texto,
        ]
        .into_iter()
        .flatten();
        cfg.chain(form)
            .any(|e| e.read(cx).focus_handle(cx).is_focused(window))
    }
}

impl Render for CerberusView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.tela {
            Tela::Configuracoes => self.garantir_config(window, cx),
            Tela::Excecoes | Tela::Testar => self.garantir_acoes(window, cx),
            _ => {}
        }
        let p = palette(cx);
        let (esquerda, direita) = self.rodape();
        let rail = self.rail(cx);
        let corpo = match self.tela {
            Tela::Vereditos => self.tela_vereditos(cx).into_any_element(),
            Tela::Guardas => self.tela_guardas(cx).into_any_element(),
            Tela::Modos => self.tela_modos(cx).into_any_element(),
            Tela::Configuracoes => self.tela_config(cx).into_any_element(),
            Tela::Excecoes => self.tela_excecoes(cx).into_any_element(),
            Tela::Testar => self.tela_testar(cx).into_any_element(),
        };
        div()
            .id("cerberus")
            .track_focus(&self.foco)
            .on_key_down(cx.listener(|v, ev: &KeyDownEvent, window, cx| {
                let campo_focado = v.campo_focado(window, cx);
                if let Some(t) = vm::tela_da_tecla(&ev.keystroke.key, campo_focado) {
                    v.mudar_tela(t, cx);
                    cx.notify();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(p.bg)
            .text_color(p.fg)
            .child(div().flex_1().flex().min_h_0().child(rail).child(corpo))
            .child(
                StatusBar::new(esquerda, direita)
                    .version(concat!("cerberus ", env!("CARGO_PKG_VERSION"))),
            )
    }
}

pub fn abrir_janela(cx: &mut App) {
    abrir_janela_em(cx, Tela::Vereditos);
}

/// Abre a janela já na tela pedida (`--tela`).
pub fn abrir_janela_em(cx: &mut App, tela: Tela) {
    let aberta = cx.open_window(gpui::WindowOptions::default(), |window, cx| {
        window.set_window_title(crate::NOME);
        let cwd = std::env::current_dir().unwrap_or_default();
        let view: Entity<CerberusView> = cx.new(|cx| {
            CerberusView::iniciar(cx);
            let mut v = CerberusView::new(Env::from_process(), cwd, cx.focus_handle(), cx);
            if tela != Tela::Vereditos {
                v.mudar_tela(tela, cx);
            }
            v
        });
        view
    });
    if aberta.is_err() {
        cx.quit();
    }
}
