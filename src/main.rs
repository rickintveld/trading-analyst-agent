use clap::Parser;
use trade_analyst::cli::Cli;

fn main() {
    let cli = Cli::parse();
    if let Err(err) = trade_analyst::cli::run(cli) {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
