use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::{Config, SourceProvider};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn run(source: &str) -> i32 {
    let directory = std::env::temp_dir().join(format!(
        "rock_object_contextual_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let entry = directory.join("main.rk");
    rock_lib::compile(&Config {
        entry_file: entry.clone(),
        output_dir: directory.clone(),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual {
            path: entry,
            text: source.into(),
        }],
        ..Config::default()
    })
    .unwrap();
    let output = std::process::Command::new(directory.join("main"))
        .output()
        .unwrap();
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let code = output.status.code().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    code
}

#[test]
fn existential_words_remain_ordinary_function_names_outside_their_forms() {
    assert_eq!(
        run(r#"
open = value -> value
pack = left, right -> ~I64Add left, right
exists = value -> value
same_type = value -> value
main = -> pack (open (same_type 40)), (exists 2)
"#),
        42
    );
}

#[test]
fn existential_words_remain_ordinary_member_and_field_names() {
    assert_eq!(
        run(r#"
struct Number
    < pack: I64
    < exists: I64
impl Number
    @open = -> self.pack
    @same_type = other -> ~I64Add self.exists, other
main = ->
    number = Number
        pack: 20
        exists: 20
    ~I64Add (number.same_type (number.open!)), 2
"#),
        42
    );
}
