mod cli;
mod constants;
mod dev;
mod fsutil;
mod home;
mod layout;
mod release;
mod selection;
mod shell;
mod target;
mod toolchain;
#[cfg(windows)]
mod user_path;

#[cfg(test)]
mod tests;

fn main() {
    // Installed as `rock`, `rockc` and `rock-lsp` too, like rustup's proxies:
    // the name it was started under selects the toolchain binary to run.
    let mut args = std::env::args_os();
    let invoked = args.next();
    if let Some(binary) = invoked.as_deref().and_then(proxied_binary) {
        let arguments: Vec<_> = args.collect();
        let result = home::RockupHome::resolve()
            .and_then(|home| toolchain::proxy_toolchain_command(&home, binary, &arguments));
        match result {
            Ok(status) => std::process::exit(fsutil::exit_code(status)),
            Err(error) => {
                eprintln!("Error: {}", error);
                std::process::exit(1);
            }
        }
    }
    match cli::run() {
        Ok(cli::Exit::Success) => {}
        Ok(cli::Exit::Code(code)) => std::process::exit(code),
        Err(error) => {
            eprintln!("Error: {}", error);
            std::process::exit(1);
        }
    }
}

/// The toolchain binary a proxy name selects, from `argv[0]`.
fn proxied_binary(invoked: &std::ffi::OsStr) -> Option<&'static str> {
    let name = std::path::Path::new(invoked).file_stem()?;
    [
        constants::ROCK_BIN_NAME,
        constants::ROCKC_BIN_NAME,
        constants::ROCK_LSP_BIN_NAME,
    ]
    .into_iter()
    .find(|binary| name == *binary)
}
