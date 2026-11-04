use super::*;

impl RockcInvocation {
    pub(crate) fn args_as_strings(&self) -> Vec<String> {
        self.args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }
}
