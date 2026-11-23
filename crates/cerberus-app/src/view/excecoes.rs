//! Telas de ação: Exceções (lista e formulário de abertura) e Testar guarda (ensaio sem gravar).

use std::path::PathBuf;

use gpui::{
    AnyElement, AppContext, Context, Entity, IntoElement, ParentElement, SharedString, Styled,
    Window, div, px,
};
use gpui_component::input::{InputEvent, InputState};
use hades_ui::components::{
    Align, Button, Card, DataTable, EmptyState, Page, PageHeader, PathField, Segmented,
    StatusBadge, TermBlock, TextField,
};
use hades_ui::icons::IconName;
use hades_ui::tokens::space;

use super::CerberusView;
use crate::service::{self, NovaExcecao};
use crate::vm::{self, CABECAS, COLUNAS_EXCECOES, Escolha, Tarefa, Tela};

/// Formulário da tela Exceções.
#[derive(Default)]
pub(super) struct FormExcecao {
    pub escolha: Escolha,
    pub motivo: Option<Entity<InputState>>,
    pub ttl: Option<Entity<InputState>>,
    pub alvo: Option<Entity<InputState>>,
    pub tentou: bool,
    pub recusa: Option<String>,
    pub t: Tarefa,
}

/// Formulário da tela Testar guarda.
#[derive(Default)]
pub(super) struct Ensaio {
    pub escolha: Escolha,
    pub de_arquivo: bool,
    pub texto: Option<Entity<InputState>>,
    pub arquivo: Option<PathBuf>,
    pub resultado: Option<Result<cerberus_core::modelo::Veredito, String>>,
    pub t: Tarefa,
}

type Campo = fn(&mut CerberusView) -> &mut Escolha;

fn entrada(
    window: &mut Window,
    cx: &mut Context<CerberusView>,
    valor: String,
) -> Entity<InputState> {
    let e = cx.new(|cx| InputState::new(window, cx).default_value(valor));
    cx.subscribe(&e, |_: &mut CerberusView, _, ev: &InputEvent, cx| {
        if matches!(ev, InputEvent::Change) {
            cx.notify();
        }
    })
    .detach();
    e
}

fn valor(e: &Option<Entity<InputState>>, cx: &gpui::App) -> String {
    e.as_ref()
        .map(|e| e.read(cx).value().to_string())
        .unwrap_or_default()
}

impl CerberusView {
    /// Cria os campos de texto das telas de ação; precisa da janela.
    pub(super) fn garantir_acoes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.motivo.is_none() {
            self.form.motivo = Some(entrada(window, cx, String::new()));
            self.form.alvo = Some(entrada(window, cx, String::new()));
        }
        if self.form.ttl.is_none()
            && let Some(Ok(h)) = self.dados.as_ref().map(|d| d.ttl_h.clone())
        {
            self.form.ttl = Some(entrada(window, cx, h.to_string()));
        }
        if self.ensaio.texto.is_none() {
            self.ensaio.texto = Some(entrada(window, cx, String::new()));
        }
    }

    /// "Criar exceção" do inspector: abre Exceções com cabeça, guarda e momento do veredito.
    pub(super) fn criar_excecao_de(
        &mut self,
        cab: &str,
        guarda: &str,
        momento: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(e) = Escolha::de(cab, guarda, momento) {
            self.form.escolha = e;
            self.form.tentou = false;
            self.form.recusa = None;
        }
        self.mudar_tela(Tela::Excecoes, cx);
    }

    /// Cabeça, guarda e momento de um formulário; `campo` diz qual.
    fn seletor(
        &self,
        id: &'static str,
        this: &Escolha,
        campo: Campo,
        so_exec: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let guardas = self
            .dados
            .as_ref()
            .map_or(&[][..], |d| d.guardas.as_slice());
        let ent = cx.entity();
        let cabecas = Segmented::new(
            SharedString::from(format!("{id}-cabeca")),
            CABECAS.map(|c| c.nome()),
        )
        .selected(CABECAS.iter().position(|c| *c == this.cabeca).unwrap_or(0))
        .on_select(move |i, _, cx| {
            ent.update(cx, |v, cx| {
                campo(v).trocar_cabeca(CABECAS[i]);
                cx.notify();
            })
        });
        let opcoes: Vec<_> = vm::guardas_da_escolha(guardas, this.cabeca, so_exec)
            .into_iter()
            .cloned()
            .collect();
        let mut tabela = DataTable::new([
            (SharedString::from("Guarda"), Align::Start),
            (SharedString::from("Momentos"), Align::Start),
        ])
        .empty(if so_exec {
            "Nenhuma guarda exec nesta cabeça"
        } else {
            "Nenhuma guarda nesta cabeça"
        });
        for g in &opcoes {
            let momentos: Vec<_> = g.momentos.iter().map(|m| m.as_str()).collect();
            tabela = tabela.row([SharedString::from(g.id.clone()), momentos.join(", ").into()]);
        }
        if let Some(p) = opcoes
            .iter()
            .position(|g| Some(&g.id) == this.guarda.as_ref())
        {
            tabela = tabela.selected(p);
        }
        let ent = cx.entity();
        let tabela = tabela.on_select(move |p, _, cx| {
            if let Some(g) = opcoes.get(p) {
                let g = g.clone();
                ent.update(cx, |v, cx| {
                    campo(v).trocar_guarda(&g);
                    cx.notify();
                });
            }
        });
        let mut coluna = div()
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .child(cabecas)
            .child(tabela);
        let momentos = vm::momentos_da_escolha(guardas, this);
        if !momentos.is_empty() {
            let ent = cx.entity();
            let atual = this
                .momento
                .as_ref()
                .and_then(|m| momentos.iter().position(|x| x == m));
            let lista = momentos.clone();
            let mut seg = Segmented::new(SharedString::from(format!("{id}-momento")), momentos);
            if let Some(p) = atual {
                seg = seg.selected(p);
            }
            coluna = coluna.child(seg.on_select(move |i, _, cx| {
                let m = lista.get(i).cloned();
                ent.update(cx, |v, cx| {
                    campo(v).momento = m;
                    cx.notify();
                })
            }));
        }
        coluna.into_any_element()
    }

    fn escolha_form(v: &mut CerberusView) -> &mut Escolha {
        &mut v.form.escolha
    }

    fn escolha_ensaio(v: &mut CerberusView) -> &mut Escolha {
        &mut v.ensaio.escolha
    }

    fn erros_do_form(&self, cx: &gpui::App) -> (Option<String>, Result<i64, String>) {
        let motivo = vm::erro_do_motivo(&valor(&self.form.motivo, cx));
        let max = self
            .dados
            .as_ref()
            .and_then(|d| d.ttl_h.clone().ok())
            .unwrap_or(720);
        (motivo, vm::ttl_em_horas(&valor(&self.form.ttl, cx), max))
    }

    fn abrir_excecao(&mut self, cx: &mut Context<Self>) {
        self.form.tentou = true;
        self.form.recusa = None;
        let (erro_motivo, ttl) = self.erros_do_form(cx);
        let e = &self.form.escolha;
        let (Some(guarda), Some(momento), None, Ok(ttl_h)) =
            (e.guarda.clone(), e.momento.clone(), erro_motivo, ttl)
        else {
            cx.notify();
            return;
        };
        let Some(g) = self.form.t.iniciar() else {
            return;
        };
        let pedido = NovaExcecao {
            cabeca: e.cabeca.as_str().to_owned(),
            guarda,
            momento,
            motivo: valor(&self.form.motivo, cx),
            alvo: vm::pares_do_alvo(&valor(&self.form.alvo, cx)),
            ttl_h,
        };
        let opcoes = service::opcoes(self.env.clone(), &self.cwd);
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { service::abrir_excecao(opcoes, pedido) })
                .await;
            let _ = this.update(cx, |v, cx| {
                if !v.form.t.concluir(g) {
                    return;
                }
                match r {
                    Ok(exc) => {
                        v.aviso = Some(format!("Exceção {} aberta", exc.id().unwrap_or("?")));
                        v.form.tentou = false;
                        v.form.motivo = None;
                        v.form.alvo = None;
                        v.forcar_recarga(cx);
                    }
                    Err(e) => v.form.recusa = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn formulario_excecao(&self, cx: &mut Context<Self>) -> Card {
        let (erro_motivo, ttl) = self.erros_do_form(cx);
        let mostrar = self.form.tentou;
        let mut cartao = Card::new()
            .title("Abrir exceção")
            .description("Libera uma vez a guarda escolhida; vence no prazo e não se remove.")
            .child(self.seletor("form", &self.form.escolha, Self::escolha_form, false, cx));
        if let Some(m) = &self.form.motivo {
            cartao = cartao.child(
                TextField::new(m)
                    .label("Motivo")
                    .error_opt(mostrar.then_some(erro_motivo).flatten()),
            );
        }
        if let Some(t) = &self.form.ttl {
            cartao = cartao.child(
                TextField::new(t)
                    .label("Validade (horas)")
                    .error_opt(ttl.err()),
            );
        }
        if let Some(a) = &self.form.alvo {
            cartao = cartao.child(TextField::new(a).label("Alvo (chave=valor, opcional)"));
        }
        if mostrar && !self.form.escolha.completa() {
            cartao = cartao.child(TermBlock::new().bad("escolha a guarda e o momento"));
        }
        if let Some(r) = &self.form.recusa {
            cartao = cartao.child(TermBlock::new().bad(r.clone()));
        }
        let ent = cx.entity();
        cartao.action(
            Button::new("abrir-excecao", "Abrir exceção")
                .primary()
                .icon(IconName::ShieldOff)
                .loading(self.form.t.carregando())
                .on_click(move |_, _, cx| ent.update(cx, |v, cx| v.abrir_excecao(cx))),
        )
    }

    pub(super) fn tela_excecoes(&self, cx: &mut Context<Self>) -> Page {
        let cabecalho = PageHeader::new("Exceções")
            .subtitle("Liberações de uso único abertas pelo dono")
            .description("Cada exceção vale para uma guarda num momento, uma vez, até o prazo.")
            .action(self.botao_recarregar(cx));
        let pagina = Page::new("pagina-excecoes").header(cabecalho);
        let Some(d) = &self.dados else {
            return pagina.child(self.esqueleto("excecoes"));
        };
        let lista: AnyElement = match &d.excecoes {
            Err(e) => EmptyState::new("Exceções ilegíveis")
                .hint(e.clone())
                .into_any_element(),
            Ok(excs) => {
                let colunas = COLUNAS_EXCECOES
                    .iter()
                    .map(|n| (SharedString::from(*n), Align::Start));
                let mut tabela = DataTable::new(colunas).empty("Nenhuma exceção");
                for e in excs.iter().rev() {
                    let l = vm::linha_excecao(e, d.agora_s);
                    tabela = tabela.row(l.celulas().into_iter().map(SharedString::from));
                }
                Card::new()
                    .title("Exceções")
                    .meta(vm::contador_abertas(vm::abertas(excs, d.agora_s)))
                    .child(tabela)
                    .into_any_element()
            }
        };
        pagina.child(
            div()
                .flex()
                .gap(px(space::LG))
                .child(div().flex().flex_col().flex_1().min_w_0().child(lista))
                .child(
                    div()
                        .flex_none()
                        .w(px(super::LARGURA_INSPECTOR))
                        .child(self.formulario_excecao(cx)),
                ),
        )
    }

    fn rodar_ensaio(&mut self, cx: &mut Context<Self>) {
        let e = &self.ensaio.escolha;
        let (Some(guarda), Some(momento)) = (e.guarda.clone(), e.momento.clone()) else {
            return;
        };
        let ctx: cerberus_core::modelo::Ctx = if self.ensaio.de_arquivo {
            let Some(p) = &self.ensaio.arquivo else {
                return;
            };
            [("arquivo".to_owned(), p.display().to_string())]
                .into_iter()
                .collect()
        } else {
            [("texto".to_owned(), valor(&self.ensaio.texto, cx))]
                .into_iter()
                .collect()
        };
        let Some(g) = self.ensaio.t.iniciar() else {
            return;
        };
        let cab = e.cabeca.as_str().to_owned();
        let opcoes = service::opcoes(self.env.clone(), &self.cwd);
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { service::ensaiar(opcoes, &cab, &guarda, &momento, ctx) })
                .await;
            let _ = this.update(cx, |v, cx| {
                if v.ensaio.t.concluir(g) {
                    v.ensaio.resultado = Some(r);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn resultado_do_ensaio(&self) -> Card {
        let cartao = Card::new()
            .title("Resultado")
            .note("O ensaio não grava veredito nem usa exceção.");
        match &self.ensaio.resultado {
            None => cartao.child(
                EmptyState::new("Nada rodado ainda").hint("Escolha uma guarda exec e rode."),
            ),
            Some(Err(e)) => cartao.child(TermBlock::new().bad(e.clone())),
            Some(Ok(v)) => {
                let r = vm::resultado_do_ensaio(v);
                let mut bloco = TermBlock::new();
                if r.evidencia.is_empty() {
                    bloco = bloco.dim("sem evidência");
                }
                for l in r.evidencia {
                    bloco = bloco.line(l);
                }
                if let Some(d) = r.detalhe {
                    bloco = bloco.dim(d);
                }
                cartao
                    .status(StatusBadge::new(r.rotulo, r.tom))
                    .child(bloco)
            }
        }
    }

    pub(super) fn tela_testar(&self, cx: &mut Context<Self>) -> Page {
        let cabecalho = PageHeader::new("Testar guarda")
            .subtitle("Roda uma guarda exec sobre um texto ou arquivo")
            .description("Mostra o veredito que a guarda daria, sem gravar no histórico.");
        let pagina = Page::new("pagina-testar").header(cabecalho);
        if self.dados.is_none() {
            return pagina.child(self.esqueleto("testar"));
        }
        let ent = cx.entity();
        let fonte = Segmented::new("ensaio-fonte", ["Texto", "Arquivo"])
            .selected(usize::from(self.ensaio.de_arquivo))
            .on_select(move |i, _, cx| {
                ent.update(cx, |v, cx| {
                    v.ensaio.de_arquivo = i == 1;
                    cx.notify();
                })
            });
        let mut entrada = Card::new()
            .title("Entrada")
            .child(self.seletor(
                "ensaio",
                &self.ensaio.escolha,
                Self::escolha_ensaio,
                true,
                cx,
            ))
            .child(fonte);
        if self.ensaio.de_arquivo {
            let ent = cx.entity();
            let mut campo = PathField::new("ensaio-arquivo").placeholder("Escolha um arquivo");
            if let Some(p) = &self.ensaio.arquivo {
                campo = campo.value(p.display().to_string());
            }
            entrada = entrada.child(campo.on_pick(move |p, cx| {
                ent.update(cx, |v, cx| {
                    v.ensaio.arquivo = Some(p);
                    cx.notify();
                })
            }));
        } else if let Some(t) = &self.ensaio.texto {
            entrada = entrada.child(TextField::new(t).label("Texto"));
        }
        let ent = cx.entity();
        let mut botao = Button::new("rodar-ensaio", "Rodar")
            .primary()
            .icon(IconName::Play)
            .loading(self.ensaio.t.carregando())
            .on_click(move |_, _, cx| ent.update(cx, |v, cx| v.rodar_ensaio(cx)));
        if !self.ensaio.escolha.completa() {
            botao = botao.disabled("escolha a guarda e o momento");
        } else if self.ensaio.de_arquivo && self.ensaio.arquivo.is_none() {
            botao = botao.disabled("escolha o arquivo");
        }
        pagina.child(
            div()
                .flex()
                .gap(px(space::LG))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(entrada.action(botao)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(self.resultado_do_ensaio()),
                ),
        )
    }
}
