use super::*;

impl Lowerer {
    pub(crate) fn fixed_array_len_from_type_name(type_name: &str) -> Option<usize> {
        let inner = type_name.strip_prefix('[')?.strip_suffix(']')?;
        let (_, len) = inner.split_once(';')?;
        len.trim().parse().ok()
    }

    pub(crate) fn receiver_type_name_matches(candidate: &str, lookup: &str) -> bool {
        if candidate == lookup {
            return true;
        }

        fn borrowed_slice_elem(name: &str) -> Option<&str> {
            name.strip_prefix("&[")
                .and_then(|name| name.strip_suffix(']'))
                .filter(|name| !name.contains(';'))
        }

        fn mut_borrowed_slice_elem(name: &str) -> Option<&str> {
            name.strip_prefix("&mut [")
                .and_then(|name| name.strip_suffix(']'))
                .filter(|name| !name.contains(';'))
        }

        let is_generic_elem = |name: &str| {
            !matches!(
                name,
                "I8" | "I16"
                    | "I32"
                    | "I64"
                    | "U8"
                    | "U16"
                    | "U32"
                    | "U64"
                    | "F32"
                    | "F64"
                    | "Bool"
                    | "Char"
                    | "Str"
            )
        };

        if let (Some(candidate_elem), Some(lookup_elem)) =
            (borrowed_slice_elem(candidate), borrowed_slice_elem(lookup))
        {
            return candidate_elem == lookup_elem || is_generic_elem(candidate_elem);
        }

        if let (Some(candidate_elem), Some(lookup_elem)) = (
            mut_borrowed_slice_elem(candidate),
            mut_borrowed_slice_elem(lookup),
        ) {
            return candidate_elem == lookup_elem || is_generic_elem(candidate_elem);
        }

        match (
            Self::fixed_array_len_from_type_name(candidate),
            Self::fixed_array_len_from_type_name(lookup),
        ) {
            (Some(candidate_len), Some(lookup_len)) => candidate_len == lookup_len,
            _ => false,
        }
    }
}
