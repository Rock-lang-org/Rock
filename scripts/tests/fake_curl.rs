// Offline stand-in for curl.exe in the Windows release test, which cannot run
// the bash transport in release.sh. Serves the release assets under test:
// `RELEASE_TEST_ASSETS` holds them, `RELEASE_TEST_TAG` names the release.
// Build: rustc --edition 2021 -O -o curl.exe scripts/tests/fake_curl.rs
use std::{env, fs, io::Write, path::Path, process::exit};

fn main() {
    let mut output = None;
    let mut url = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--output" {
            output = args.next();
        } else if arg.starts_with("https://") {
            url = Some(arg);
        }
    }
    let url = url.unwrap_or_default();
    let tag = env::var("RELEASE_TEST_TAG").expect("RELEASE_TEST_TAG");
    let assets = env::var("RELEASE_TEST_ASSETS").expect("RELEASE_TEST_ASSETS");
    let base = "https://github.com/Rock-lang-org/Rock/releases";
    if url == format!("{base}/latest") {
        print!("{base}/tag/{tag}");
        return;
    }
    let pinned = format!("{base}/download/{tag}/");
    let latest = format!("{base}/latest/download/");
    let Some(name) = url
        .strip_prefix(&pinned)
        .or_else(|| url.strip_prefix(&latest))
    else {
        eprintln!("Unexpected offline request: {url}");
        exit(1);
    };
    let bytes = match fs::read(Path::new(&assets).join(name)) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("Missing release asset {name}: {error}");
            exit(22);
        }
    };
    match output.as_deref() {
        // `rockup` discards redirect probes into NUL.
        Some(path) if !path.eq_ignore_ascii_case("NUL") => fs::write(path, bytes).unwrap(),
        Some(_) => {}
        None => std::io::stdout().write_all(&bytes).unwrap(),
    }
}
