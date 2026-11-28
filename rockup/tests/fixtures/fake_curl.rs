// A process-local stand-in for curl, compiled by releases.rs and put first on
// PATH: production has no insecure URL or fixture override. It also asserts
// the curl security flags rockup must pass. `FIXTURE` is the fixture root:
// `releases.json` answers the GitHub API and `assets/` holds the downloads.
use std::{env, fs, io::Write, path::Path, process::exit};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let joined = format!(" {} ", args.join(" "));
    for (required, code) in [
        (" --disable --fail --location ", 90),
        (" --retry 3 ", 91),
        (" --proto =https --proto-redir =https ", 92),
    ] {
        if !joined.contains(required) {
            exit(code);
        }
    }
    let fixture = env::var("FIXTURE").expect("FIXTURE");
    let fixture = Path::new(&fixture);
    let mut requests = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(fixture.join("requests"))
        .unwrap();
    writeln!(requests, "{}", args.join(" ")).unwrap();
    if env::var("FAIL_DOWNLOAD").as_deref() == Ok("yes") {
        exit(22);
    }

    let mut output = None;
    let mut latest = false;
    let mut url = String::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--output" => output = iter.next().cloned(),
            "--write-out" => {
                iter.next();
                latest = true;
            }
            arg if arg.starts_with("https://") => url = arg.to_string(),
            _ => {}
        }
    }
    let releases = "https://github.com/Rock-lang-org/Rock/releases";
    if url == "https://api.github.com/repos/Rock-lang-org/Rock/releases?per_page=100&page=1" {
        std::io::stdout()
            .write_all(&fs::read(fixture.join("releases.json")).unwrap())
            .unwrap();
    } else if latest {
        if url != format!("{releases}/latest") {
            exit(93);
        }
        print!("{releases}/tag/v1.2.3");
    } else {
        let Some(name) = url.strip_prefix(&format!("{releases}/download/v1.2.3/")) else {
            exit(93);
        };
        let output = output.expect("--output");
        fs::copy(fixture.join("assets").join(name), output).unwrap_or_else(|_| exit(22));
    }
}
