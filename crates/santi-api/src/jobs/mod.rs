mod files;
mod model;
mod process;
mod systemd;

pub use systemd::Systemd;

pub fn run() -> Result<(), String> {
    process::run()
}

pub fn finalize() -> Result<(), String> {
    process::finalize()
}
