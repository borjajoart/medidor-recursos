//! Ejecutable de consola.

use medidor_recursos::cli::{self, Command, HELP};
use medidor_recursos::monitor;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args) {
        Err(e) => {
            eprintln!("Error: {e}\n\n{HELP}");
            std::process::exit(2);
        }
        Ok(Command::Help) => println!("{HELP}"),
        Ok(Command::Stop) => std::process::exit(monitor::request_stop()),
        Ok(Command::Status) => std::process::exit(monitor::print_status()),
        Ok(Command::Run { background: true, child: false, .. }) => match monitor::spawn_child(&args) {
            Ok(pid) => {
                println!("Monitor iniciado en segundo plano (PID {pid}).");
                println!("Para terminar y generar el informe: medidor-recursos detener");
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        Ok(Command::Run { cfg, .. }) => {
            if let Err(e) = std::fs::create_dir_all(&cfg.out_dir) {
                eprintln!("Error: no se puede crear la carpeta {}: {e}", cfg.out_dir.display());
                std::process::exit(2);
            }
            monitor::run(cfg);
        }
    }
}
