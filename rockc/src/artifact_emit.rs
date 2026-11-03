use std::path::PathBuf;

pub(super) fn normalize_product_object_path(
    artifact_path: &std::path::Path,
    products: &mut rock_lib::products::CompilerProducts,
) {
    let Some(object_path) = products.link.object_path.as_ref() else {
        return;
    };
    let Some(object_file_name) = object_path.file_name() else {
        return;
    };
    let Some(artifact_parent) = artifact_path.parent() else {
        return;
    };
    let Some(object_parent) = object_path.parent() else {
        return;
    };

    if same_directory(artifact_parent, object_parent) {
        products.link.object_path = Some(PathBuf::from(object_file_name));
        return;
    }

    if let Some(sibling_root) = artifact_parent.parent() {
        if let Some(relative_path) = relative_to_root(object_path, sibling_root) {
            products.link.object_path = Some(relative_path);
        }
    }
}

fn relative_to_root(path: &std::path::Path, root: &std::path::Path) -> Option<PathBuf> {
    if let Ok(relative) = path.strip_prefix(root) {
        if !relative.as_os_str().is_empty() {
            return Some(relative.to_path_buf());
        }
    }

    let path = path.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    path.strip_prefix(root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
}

fn same_directory(left: &std::path::Path, right: &std::path::Path) -> bool {
    if left == right {
        return true;
    }

    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
