//! Tabela dos vereditos com a coluna Veredito colorida pelo tom. O `DataTable` do Hades-UI só aceita célula de texto
//! e pede, para célula com mais que texto, linhas montadas com `div` nas mesmas cores da paleta: aqui o mesmo
//! desenho (cabeçalho 40 px em caixa alta, zebra, hover, seleção), com o tom da paleta na célula.

use std::rc::Rc;

use gpui::{
    App, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use hades_ui::tokens::size;
use hades_ui::{Tone, palette};

type AoEscolher = Rc<dyn Fn(usize, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(super) struct TabelaTonada {
    id: ElementId,
    colunas: Vec<SharedString>,
    linhas: Vec<Vec<(SharedString, Option<Tone>)>>,
    selecionada: Option<usize>,
    vazio: SharedString,
    ao_escolher: Option<AoEscolher>,
}

impl TabelaTonada {
    pub fn new(
        id: impl Into<ElementId>,
        colunas: impl IntoIterator<Item = SharedString>,
        vazio: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            colunas: colunas.into_iter().collect(),
            linhas: Vec::new(),
            selecionada: None,
            vazio: vazio.into(),
            ao_escolher: None,
        }
    }

    pub fn linha(
        mut self,
        celulas: impl IntoIterator<Item = (SharedString, Option<Tone>)>,
    ) -> Self {
        self.linhas.push(celulas.into_iter().collect());
        self
    }

    pub fn selecionada(mut self, i: usize) -> Self {
        self.selecionada = Some(i);
        self
    }

    pub fn ao_escolher(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.ao_escolher = Some(Rc::new(f));
        self
    }
}

fn celula(texto: SharedString) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .px(px(8.0))
        .py(px(7.0))
        .child(texto)
}

impl RenderOnce for TabelaTonada {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let mut cabecalho = div()
            .flex()
            .items_center()
            .h(px(40.0))
            .border_b_1()
            .border_color(p.border)
            .text_color(p.muted_fg)
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(size::MICRO));
        for titulo in self.colunas {
            cabecalho = cabecalho.child(celula(titulo.to_uppercase().into()));
        }
        let mut corpo = div().flex().flex_col().flex_1();
        if self.linhas.is_empty() {
            corpo = corpo.child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .min_h(px(80.0))
                    .p(px(24.0))
                    .text_color(p.muted_fg)
                    .child(self.vazio),
            );
        }
        for (i, linha) in self.linhas.into_iter().enumerate() {
            let mut fila = div().flex().border_b_1().border_color(p.border);
            if i % 2 == 1 {
                fila = fila.bg(p.muted.opacity(0.55));
            }
            if self.selecionada == Some(i) {
                fila = fila.bg(p.accent);
            }
            for (texto, tom) in linha {
                let c = celula(texto);
                fila = fila.child(match tom {
                    Some(tom) => c.text_color(p.tone(tom)).font_weight(FontWeight::SEMIBOLD),
                    None => c,
                });
            }
            let fila = fila.id(ElementId::NamedInteger("linha".into(), i as u64));
            corpo = corpo.child(match self.ao_escolher.clone() {
                Some(f) => fila
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.hover))
                    .on_click(move |_, window, cx| f(i, window, cx))
                    .into_any_element(),
                None => fila.into_any_element(),
            });
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .text_color(p.fg)
            .text_size(px(size::LABEL))
            .child(cabecalho)
            .child(
                div()
                    .id(self.id)
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(corpo),
            )
    }
}
