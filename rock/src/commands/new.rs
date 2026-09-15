use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn new_project(parent: &Path, name: &str) -> Result<PathBuf, String> {
    if !name.starts_with(|c: char| c.is_ascii_alphabetic())
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("Project names must start with an ASCII letter and contain only ASCII letters, digits, '-' or '_'".to_string());
    }
    if name == "stdlib" {
        return Err("Project name 'stdlib' is reserved for the standard library".to_string());
    }

    let root = parent.join(name);
    // create_dir refuses existing destinations, including symlinks.
    fs::create_dir(&root).map_err(|e| {
        format!(
            "Failed to create project directory {}: {}",
            root.display(),
            e
        )
    })?;
    let manifest = format!(
        "[crate]\nname = \"{}\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/main.rk\"\n",
        name
    );
    fs::create_dir(root.join("src")).map_err(|e| {
        format!(
            "Failed to create source directory in {}: {}",
            root.display(),
            e
        )
    })?;
    for (path, content) in [
        ("rock.toml", manifest.as_str()),
        ("src/main.rk", "main = !->\n    \"Hello, Rock!\".println!\n"),
        (".gitignore", "/build/\n"),
    ] {
        let path = root.join(path);
        fs::write(&path, content)
            .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
    }
    Ok(root)
}
