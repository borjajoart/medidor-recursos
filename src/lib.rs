//! Medidor de recursos: monitor ligero de procesos, temperaturas y FPS para Windows.
//! Lo usan dos ejecutables: `medidor-recursos` (consola) y `medidor-gui` (interfaz).

pub mod analysis;
pub mod cli;
pub mod compare;
pub mod fps;
pub mod gpu_proc;
pub mod monitor;
pub mod net;
pub mod report;
pub mod sensors;

pub use cli::Config;
