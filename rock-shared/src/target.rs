//! Compilation target identity.
//!
//! One resolved [`Target`] drives cfg selection, LLVM emission, linking and
//! artifact compatibility. Host is where the compiler runs; target is where
//! the produced program runs.

/// Built-in cfg keys and the values each currently recognizes. Unknown keys or
/// values are diagnostics, so a typo cannot silently disable a backend.
pub const CFG_KEYS: &[(&str, &[&str])] = &[
    ("target_os", &["linux", "macos", "windows"]),
    ("target_family", &["unix", "windows"]),
    ("target_arch", &["x86_64", "aarch64"]),
    ("target_env", &["gnu", "msvc", ""]),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub triple: String,
    pub arch: String,
    pub os: TargetOs,
    pub env: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOs {
    Linux,
    MacOs,
    Windows,
}

impl Target {
    /// Parses a supported triple such as `x86_64-unknown-linux-gnu`,
    /// `aarch64-apple-darwin` or `x86_64-pc-windows-msvc`.
    pub fn parse(triple: &str) -> Result<Self, String> {
        let parts: Vec<&str> = triple.split('-').collect();
        let unsupported = || {
            format!(
                "unsupported target triple '{}'; supported targets: \
                 x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, \
                 aarch64-apple-darwin, x86_64-apple-darwin, x86_64-pc-windows-msvc",
                triple
            )
        };
        let arch = match parts.first() {
            Some(&"x86_64") => "x86_64",
            Some(&"aarch64") | Some(&"arm64") => "aarch64",
            _ => return Err(unsupported()),
        };
        // Aliases (`arm64`, versioned Darwin, other vendors) normalize to one
        // canonical triple: sysroot paths, build directories and artifact
        // compatibility all compare triples as strings.
        let (os, env, canonical) = match parts.as_slice() {
            [_, _, "linux", "gnu"] => (
                TargetOs::Linux,
                "gnu",
                format!("{}-unknown-linux-gnu", arch),
            ),
            [_, "apple", os] if os.starts_with("darwin") || os.starts_with("macos") => {
                (TargetOs::MacOs, "", format!("{}-apple-darwin", arch))
            }
            [_, _, "windows", "msvc"] => (
                TargetOs::Windows,
                "msvc",
                format!("{}-pc-windows-msvc", arch),
            ),
            _ => return Err(unsupported()),
        };
        Ok(Self {
            triple: canonical,
            arch: arch.to_string(),
            os,
            env: env.to_string(),
        })
    }

    /// The machine running the compiler; falls back to the Linux baseline
    /// when the host is not a supported target.
    pub fn host() -> Self {
        Self::parse(&crate::sysroot::host_target_triple())
            .unwrap_or_else(|_| Self::parse("x86_64-unknown-linux-gnu").expect("baseline target"))
    }

    pub fn os_name(&self) -> &'static str {
        match self.os {
            TargetOs::Linux => "linux",
            TargetOs::MacOs => "macos",
            TargetOs::Windows => "windows",
        }
    }

    pub fn family(&self) -> &'static str {
        match self.os {
            TargetOs::Linux | TargetOs::MacOs => "unix",
            TargetOs::Windows => "windows",
        }
    }

    /// Value of a built-in cfg key for this target, or `None` for unknown keys.
    pub fn cfg_value(&self, key: &str) -> Option<&str> {
        match key {
            "target_os" => Some(self.os_name()),
            "target_family" => Some(self.family()),
            "target_arch" => Some(&self.arch),
            "target_env" => Some(&self.env),
            _ => None,
        }
    }

    /// Every built-in cfg pair, as printed by `rockc --print cfg`.
    pub fn cfg_pairs(&self) -> Vec<(&'static str, String)> {
        CFG_KEYS
            .iter()
            .map(|(key, _)| (*key, self.cfg_value(key).unwrap_or_default().to_string()))
            .collect()
    }

    pub fn exe_suffix(&self) -> &'static str {
        match self.os {
            TargetOs::Windows => ".exe",
            _ => "",
        }
    }

    pub fn object_extension(&self) -> &'static str {
        match self.os {
            TargetOs::Windows => "obj",
            _ => "o",
        }
    }

    pub fn is_host(&self) -> bool {
        *self == Self::host()
    }
}

/// Checks that `key = "value"` uses the built-in vocabulary.
pub fn validate_cfg_pair(key: &str, value: &str) -> Result<(), String> {
    let Some((_, values)) = CFG_KEYS.iter().find(|(k, _)| *k == key) else {
        let keys: Vec<&str> = CFG_KEYS.iter().map(|(k, _)| *k).collect();
        return Err(format!(
            "unknown cfg key '{}'; expected one of: {}",
            key,
            keys.join(", ")
        ));
    };
    if values.contains(&value) {
        Ok(())
    } else {
        let shown: Vec<String> = values.iter().map(|v| format!("\"{}\"", v)).collect();
        Err(format!(
            "unknown value \"{}\" for cfg key '{}'; expected one of: {}",
            value,
            key,
            shown.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests;
