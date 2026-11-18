//! Medidor de recursos: monitor ligero de procesos y temperaturas para Windows.
//! Lo usan dos ejecutables: `medidor-recursos` (consola) y `medidor-gui` (interfaz).

pub mod cli;
pub mod monitor;
pub mod report;
pub mod sensors;

pub use cli::Config;
