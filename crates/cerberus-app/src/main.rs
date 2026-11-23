use cerberus_app::vm::Tela;
use hades_ui::ThemeMode;

fn valor_de(flag: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let modo = if std::env::args().any(|a| a == "--dark") {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    let tela = valor_de("--tela")
        .and_then(|n| Tela::de_nome(&n))
        .unwrap_or(Tela::Vereditos);
    gpui_platform::application().run(move |cx| {
        hades_ui::init(cx, modo);
        cerberus_app::abrir_janela_em(cx, tela);
    });
}
