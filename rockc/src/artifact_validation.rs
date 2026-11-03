use std::path::{Path, PathBuf};

use rock_lib::{
    diagnostic::Diagnostics,
    products::{PortableProductArtifact, ProductArtifactHeader},
};

pub(super) fn validate_artifact(path: &std::path::Path) -> Result<(), Diagnostics> {
    let header = ProductArtifactHeader::read_from_path_bounded(path).map_err(|error| {
        diagnostics_from_artifact_message(path, format!("Failed to read artifact: {error}"))
    })?;
    let products = PortableProductArtifact::read_payload_from_path_bounded(path, &header)
        .and_then(PortableProductArtifact::into_products)
        .map_err(|error| {
            diagnostics_from_artifact_message(
                path,
                format!("Failed to read artifact payload: {error}"),
            )
        })?;
    let Some(object_path) = products.link.object_path else {
        return Err(diagnostics_from_artifact_message(
            path,
            format!(
                "Product artifact {} does not reference an object file",
                path.display()
            ),
        ));
    };
    let object_path = resolve_artifact_object_path(path, object_path);
    if !object_path.exists() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.push(rock_lib::diagnostic::Diagnostic::for_file(
            format!(
                "Product artifact {} references missing object file {}",
                path.display(),
                object_path.display()
            ),
            object_path,
        ));
        return Err(diagnostics);
    }

    Ok(())
}

pub(super) fn diagnostics_from_artifact_message(
    path: &Path,
    message: impl Into<String>,
) -> Diagnostics {
    let mut diagnostics = Diagnostics::default();
    diagnostics.push(rock_lib::diagnostic::Diagnostic::for_artifact(
        message.into(),
        path.to_path_buf(),
    ));
    diagnostics
}

pub(super) fn resolve_artifact_object_path(
    artifact_path: &std::path::Path,
    object_path: PathBuf,
) -> PathBuf {
    if object_path.is_absolute() {
        return object_path;
    }

    let artifact_parent = artifact_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let artifact_relative = artifact_parent.join(&object_path);
    if artifact_relative.exists() {
        return artifact_relative;
    }

    if let Some(parent) = artifact_parent.parent() {
        let sibling_relative = parent.join(&object_path);
        if sibling_relative.exists() {
            return sibling_relative;
        }
    }

    if object_path.exists() {
        return object_path;
    }

    artifact_relative
}
