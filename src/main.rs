use clap::Parser;
use herdr_talon::Cli;

fn main() {
    let cli = Cli::parse();
    if let Err(error) = herdr_talon::run(cli.command) {
        eprintln!("Talon: {error:#}");
        std::process::exit(1);
    }
}
