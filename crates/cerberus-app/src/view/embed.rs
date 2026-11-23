//! O Cerberus dentro do painel do Hades: telas do trilho, comandos da paleta e a view sem trilho nem janela.

use std::path::PathBuf;

use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, Window, div};
use hades_core::Env;
use hades_ui::{Comando, Embutida, FerramentaEmbutida, FerramentaInfo, IconName, TelaInfo};

use super::CerberusView;
use crate::vm::{self, Filtro, Tela};

const fn tela_info(t: Tela) -> TelaInfo {
    TelaInfo {
        id: t.id(),
        rotulo: t.rotulo(),
        icone: t.icone(),
    }
}

/// As telas do trilho próprio, na mesma ordem.
const TELAS: &[TelaInfo] = &[
    tela_info(Tela::Vereditos),
    tela_info(Tela::Guardas),
    tela_info(Tela::Modos),
    tela_info(Tela::Excecoes),
    tela_info(Tela::Testar),
    tela_info(Tela::Configuracoes),
];

/// As telas quando as Configurações vêm do painel.
const TELAS_INTEGRADO: &[TelaInfo] = &[
    tela_info(Tela::Vereditos),
    tela_info(Tela::Guardas),
    tela_info(Tela::Modos),
    tela_info(Tela::Excecoes),
    tela_info(Tela::Testar),
];

const RECARREGAR: &str = "recarregar";
const FILTRO_TODOS: &str = "vereditos.filtro-todos";
const FILTRO_BARRADOS: &str = "vereditos.filtro-barrados";
const FILTRO_AVISOS: &str = "vereditos.filtro-avisos";
const CRIAR_EXCECAO: &str = "vereditos.criar-excecao";
const ABRIR_EXCECAO: &str = "excecoes.abrir";
const RODAR_ENSAIO: &str = "testar.rodar";

const fn comando(
    id: &'static str,
    grupo: &'static str,
    rotulo: &'static str,
    descricao: &'static str,
    icone: IconName,
) -> Comando {
    Comando {
        id,
        grupo,
        rotulo,
        descricao: Some(descricao),
        icone,
        atalho: None,
    }
}

/// Os botões e filtros das telas.
const COMANDOS: &[Comando] = &[
    comando(
        RECARREGAR,
        "Vereditos",
        "Recarregar",
        "Relê o histórico, o registro e os modos",
        IconName::RefreshCw,
    ),
    comando(
        FILTRO_TODOS,
        "Vereditos",
        "Mostrar todos os vereditos",
        "Filtro Todos",
        IconName::List,
    ),
    comando(
        FILTRO_BARRADOS,
        "Vereditos",
        "Mostrar só os barrados",
        "Filtro Barrados",
        IconName::OctagonX,
    ),
    comando(
        FILTRO_AVISOS,
        "Vereditos",
        "Mostrar só os avisos",
        "Filtro Avisos",
        IconName::TriangleAlert,
    ),
    comando(
        CRIAR_EXCECAO,
        "Vereditos",
        "Criar exceção",
        "Abre o formulário com a guarda do veredito selecionado",
        IconName::ShieldOff,
    ),
    comando(
        ABRIR_EXCECAO,
        "Exceções",
        "Abrir exceção",
        "Grava a exceção do formulário",
        IconName::ShieldOff,
    ),
    comando(
        RODAR_ENSAIO,
        "Testar guarda",
        "Rodar o ensaio",
        "Roda a guarda escolhida sobre o texto ou o arquivo",
        IconName::Play,
    ),
];

/// Telas que a view mostra com as opções `opts`.
pub fn telas(opts: Embutida) -> &'static [TelaInfo] {
    if opts.integrado {
        TELAS_INTEGRADO
    } else {
        TELAS
    }
}

impl CerberusView {
    /// Tela de id `id` entre as visíveis; id desconhecido, ou Configurações com o painel integrado, dá a primeira.
    fn tela_visivel(&self, id: &str) -> Tela {
        telas(Embutida {
            integrado: self.integrado,
        })
        .iter()
        .find(|t| t.id == id)
        .and_then(|t| Tela::de_nome(t.id))
        .unwrap_or(Tela::Vereditos)
    }

    /// "Criar exceção" do inspector, quando há um veredito selecionado no filtro atual.
    fn criar_excecao_da_selecao(&mut self, cx: &mut Context<Self>) {
        let visiveis = self.visiveis();
        let hist = self.historico();
        let Some(p) = vm::posicao_da_selecao(hist, &visiveis, self.selecionado.as_deref()) else {
            return;
        };
        let reg = &hist[visiveis[p]];
        let [cab, guarda, momento] =
            ["cabeca", "guarda", "momento"].map(|k| vm::texto(reg, k).to_owned());
        self.criar_excecao_de(&cab, &guarda, &momento, cx);
    }

    /// "Rodar" do Testar guarda, com as mesmas condições do botão.
    fn rodar_ensaio_se_pronto(&mut self, cx: &mut Context<Self>) {
        let pronto = self.ensaio.escolha.completa()
            && !(self.ensaio.de_arquivo && self.ensaio.arquivo.is_none());
        if pronto {
            self.rodar_ensaio(cx);
        }
    }
}

impl FerramentaEmbutida for CerberusView {
    /// Ambiente do processo e pasta do projeto.
    type Ambiente = (Env, PathBuf);

    fn info() -> FerramentaInfo {
        FerramentaInfo {
            id: "cerberus",
            titulo: crate::NOME,
            icone: IconName::Cerberus,
            telas: TELAS,
            comandos: COMANDOS,
        }
    }

    fn embutida(
        (env, cwd): Self::Ambiente,
        foco: Option<&str>,
        opts: Embutida,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::iniciar(cx);
        let mut v = Self::new(env, cwd, cx.focus_handle(), cx);
        v.integrado = opts.integrado;
        if let Some(id) = foco {
            let t = v.tela_visivel(id);
            v.mudar_tela(t, cx);
        }
        v
    }

    fn render_tela(
        &mut self,
        tela: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = self.tela_visivel(tela);
        if t != self.tela {
            self.mudar_tela(t, cx);
        }
        let corpo = self.corpo(t, window, cx);
        div()
            .size_full()
            .flex()
            .min_h_0()
            .child(corpo)
            .into_any_element()
    }

    fn executar(&mut self, comando: &str, window: &mut Window, cx: &mut Context<Self>) {
        match comando {
            RECARREGAR => self.forcar_recarga(cx),
            FILTRO_TODOS | FILTRO_BARRADOS | FILTRO_AVISOS => {
                self.filtro = match comando {
                    FILTRO_BARRADOS => Filtro::Barrados,
                    FILTRO_AVISOS => Filtro::Avisos,
                    _ => Filtro::Todos,
                };
                cx.notify();
            }
            CRIAR_EXCECAO => self.criar_excecao_da_selecao(cx),
            ABRIR_EXCECAO => {
                self.garantir_acoes(window, cx);
                self.abrir_excecao(cx);
            }
            RODAR_ENSAIO => {
                self.garantir_acoes(window, cx);
                self.rodar_ensaio_se_pronto(cx);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_sem_ids_repetidos() {
        assert!(CerberusView::info().ids_repetidos().is_empty());
    }

    #[test]
    fn toda_tela_do_trilho_tem_tela_info() {
        let info = CerberusView::info();
        for t in Tela::TODAS {
            let tela = info
                .tela(t.id())
                .unwrap_or_else(|| panic!("sem TelaInfo: {}", t.rotulo()));
            assert_eq!((tela.rotulo, tela.icone), (t.rotulo(), t.icone()));
        }
        assert_eq!(info.telas.len(), Tela::TODAS.len());
    }

    #[test]
    fn ids_das_telas_valem_na_flag_tela() {
        for t in CerberusView::info().telas {
            assert_eq!(Tela::de_nome(t.id).map(Tela::id), Some(t.id));
        }
    }

    #[test]
    fn integrado_tira_configuracoes() {
        let ids = |opts| telas(opts).iter().map(|t| t.id).collect::<Vec<_>>();
        assert!(ids(Embutida::default()).contains(&Tela::Configuracoes.id()));
        let integrado = ids(Embutida { integrado: true });
        assert!(!integrado.contains(&Tela::Configuracoes.id()));
        assert_eq!(integrado.len(), Tela::TODAS.len() - 1);
    }
}
