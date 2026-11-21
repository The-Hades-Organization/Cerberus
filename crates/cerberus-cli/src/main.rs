use clap::Parser;

/// Cerberus: ferramenta do Hades, reescrita em Rust.
#[derive(Parser)]
#[command(name = "cerberus", version, about)]
struct Cli {}

fn main() {
    Cli::parse();
}
