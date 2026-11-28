// A stand-in toolchain binary for rockup's unit tests, compiled once by
// support.rs. Its behavior comes from `<stem>.fake` beside the executable, so
// one binary serves every test and is copied along with a toolchain:
//
//   log-arg1 PATH        write the first argument to PATH
//   log-env NAME PATH    write environment variable NAME to PATH
//   products PATH        write the --emit-object and --emit-artifact values to
//                        PATH, one per line, and create both files
//   exit N               exit with status N (default 0)
use std::{env, fs, path::Path, process::exit};

fn main() {
    let config = env::current_exe().unwrap().with_extension("fake");
    let config = fs::read_to_string(&config).unwrap_or_default();
    let args: Vec<String> = env::args().skip(1).collect();
    let mut code = 0;
    for line in config.lines() {
        let words: Vec<&str> = line.splitn(3, ' ').collect();
        match words.as_slice() {
            ["log-arg1", path] => fs::write(path, args.first().map_or("", String::as_str)).unwrap(),
            ["log-env", name, path] => fs::write(path, env::var(name).unwrap_or_default()).unwrap(),
            ["products", path] => {
                let value_after = |flag: &str| {
                    args.iter()
                        .position(|arg| arg == flag)
                        .and_then(|index| args.get(index + 1))
                        .cloned()
                        .unwrap_or_default()
                };
                let object = value_after("--emit-object");
                let artifact = value_after("--emit-artifact");
                fs::write(path, format!("{object}\n{artifact}\n")).unwrap();
                for product in [&object, &artifact] {
                    if let Some(parent) = Path::new(product).parent() {
                        fs::create_dir_all(parent).unwrap();
                    }
                    fs::write(product, "").unwrap();
                }
            }
            ["exit", status] => code = status.parse().unwrap(),
            _ => panic!("unknown fake tool instruction: {line}"),
        }
    }
    exit(code);
}
