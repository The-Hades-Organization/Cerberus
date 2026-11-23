# Cerberus

Cerberus: ferramenta do Hades, reescrita em Rust.

## Crates

- `crates/cerberus-core`: regra de negócio, sem interface.
- `crates/cerberus-cli`: binário `cerberus` (clap).
- `crates/cerberus-app`: view gpui exportada e binário `cerberus-app`.

## Estado

- **CLI:** o binário `cerberus` cobre os comandos do Cerberus do Hades. A paridade com a versão em TypeScript roda no
  CI e trava o merge quando diverge: CLI 78 casos, Porta 66, Limiar 106, Ensaio 3 e Vigília 105.
- **App:** o `cerberus-app` tem as 6 telas (Vereditos, Guardas, Modos, Exceções, Testar guarda e Configurações).
- **Ligar no Hades:** com `HADES_CERBERUS_IMPL=rust` (Configurações, grupo Cerberus), o `hades cerberus …` passa a chamar
  o binário `cerberus` do `HADES_CERBERUS_BIN` ou do PATH. O padrão `ts` segue usando a versão em TypeScript.

## Capturas

O `cerberus-app` nos temas claro e escuro. Os dados são fictícios, montados num HOME de exemplo.

| Tela | Claro | Escuro |
| --- | --- | --- |
| Vereditos, com o inspector aberto num barrado e a evidência mascarada | ![Vereditos, tema claro](docs/identidade/cerberus-vereditos-claro.png) | ![Vereditos, tema escuro](docs/identidade/cerberus-vereditos-escuro.png) |
| Guardas, agrupadas por cabeça | ![Guardas, tema claro](docs/identidade/cerberus-guardas-claro.png) | ![Guardas, tema escuro](docs/identidade/cerberus-guardas-escuro.png) |
| Modos, com o valor efetivo e a origem | ![Modos, tema claro](docs/identidade/cerberus-modos-claro.png) | ![Modos, tema escuro](docs/identidade/cerberus-modos-escuro.png) |
| Exceções, com o formulário de abertura preenchido | ![Exceções, tema claro](docs/identidade/cerberus-excecoes-claro.png) | ![Exceções, tema escuro](docs/identidade/cerberus-excecoes-escuro.png) |
| Testar guarda, com o veredito de um ensaio | ![Testar guarda, tema claro](docs/identidade/cerberus-testar-claro.png) | ![Testar guarda, tema escuro](docs/identidade/cerberus-testar-escuro.png) |
| Configurações | ![Configurações, tema claro](docs/identidade/cerberus-configuracoes-claro.png) | ![Configurações, tema escuro](docs/identidade/cerberus-configuracoes-escuro.png) |

## Relação com o Hades-Skill

Faz parte da reescrita em Rust das ferramentas do [Hades-Skill](https://github.com/The-Hades-Organization/Hades-Skill). O Hades-Skill continua sendo a fonte até o corte, quando passa a chamar o binário.

## Licença

MIT. Veja `LICENSE`.
