use crate::codegen::CodeGen;

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => unreachable!("hex digit must be in 0..=15"),
    }
}

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn mangle_function_symbol(name: &str) -> String {
        if name == "main" {
            return name.to_string();
        }

        let mut mangled = String::with_capacity(name.len() * 2);
        for (index, segment) in name.split("::").enumerate() {
            if index > 0 {
                mangled.push('_');
            }
            mangled.push('s');
            mangled.push_str(&segment.len().to_string());
            mangled.push('_');
            for byte in segment.as_bytes() {
                mangled.push(hex_digit(byte >> 4));
                mangled.push(hex_digit(byte & 0x0f));
            }
        }
        mangled
    }

    pub(crate) fn unique_rock_function_symbol_name(&self, desired: &str) -> String {
        if self.module.get_function(desired).is_none() {
            return desired.to_string();
        }

        let mut index = 0;
        loop {
            let candidate = if index == 0 {
                format!("__rock_{}", desired)
            } else {
                format!("__rock_{}_{}", desired, index)
            };
            if self.module.get_function(&candidate).is_none() {
                return candidate;
            }
            index += 1;
        }
    }

    pub(crate) fn sanitize_symbol(name: &str) -> String {
        name.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    }
}
