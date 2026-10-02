use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Kind {
    Type,
    Arrow(Box<Kind>, Box<Kind>),
}

impl Kind {
    pub fn arrow(input: Kind, output: Kind) -> Self {
        Self::Arrow(Box::new(input), Box::new(output))
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Kind::Type => f.write_str("Type"),
            Kind::Arrow(input, output) => {
                if matches!(input.as_ref(), Kind::Arrow(_, _)) {
                    write!(f, "({input}) -> {output}")
                } else {
                    write!(f, "{input} -> {output}")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Kind;

    #[test]
    fn kind_display_is_right_associative() {
        assert_eq!(
            Kind::arrow(Kind::Type, Kind::arrow(Kind::Type, Kind::Type)).to_string(),
            "Type -> Type -> Type"
        );
        assert_eq!(
            Kind::arrow(Kind::arrow(Kind::Type, Kind::Type), Kind::Type).to_string(),
            "(Type -> Type) -> Type"
        );
    }
}
