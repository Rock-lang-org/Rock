use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::ids::{AssocTypeId, VariantId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LanguageItemRole {
    Sized,
    Drop,
    Fold,
    Index,
    IndexMut,
    FnOnce,
    FnMut,
    Fn,
    Send,
    Sync,
    Try,
    FromResidual,
    ControlFlow,
    Range,
    Method,
    Output,
    Residual,
    Branch,
    Break,
    Continue,
    RangeFull,
    RangeFrom,
    RangeTo,
    RangeToInclusive,
    RangeExclusive,
    RangeInclusive,
}

impl LanguageItemRole {
    pub const ALL_NAMES: [&str; 26] = [
        "sized",
        "drop",
        "fold",
        "index",
        "index_mut",
        "fn_once",
        "fn_mut",
        "fn",
        "send",
        "sync",
        "try",
        "from_residual",
        "control_flow",
        "range",
        "method",
        "output",
        "residual",
        "branch",
        "break",
        "continue",
        "range_full",
        "range_from",
        "range_to",
        "range_to_inclusive",
        "range_exclusive",
        "range_inclusive",
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sized => "sized",
            Self::Drop => "drop",
            Self::Fold => "fold",
            Self::Index => "index",
            Self::IndexMut => "index_mut",
            Self::FnOnce => "fn_once",
            Self::FnMut => "fn_mut",
            Self::Fn => "fn",
            Self::Send => "send",
            Self::Sync => "sync",
            Self::Try => "try",
            Self::FromResidual => "from_residual",
            Self::ControlFlow => "control_flow",
            Self::Range => "range",
            Self::Method => "method",
            Self::Output => "output",
            Self::Residual => "residual",
            Self::Branch => "branch",
            Self::Break => "break",
            Self::Continue => "continue",
            Self::RangeFull => "range_full",
            Self::RangeFrom => "range_from",
            Self::RangeTo => "range_to",
            Self::RangeToInclusive => "range_to_inclusive",
            Self::RangeExclusive => "range_exclusive",
            Self::RangeInclusive => "range_inclusive",
        }
    }
}

impl FromStr for LanguageItemRole {
    type Err = String;

    fn from_str(role: &str) -> Result<Self, Self::Err> {
        match role {
            "sized" => Ok(Self::Sized),
            "drop" => Ok(Self::Drop),
            "fold" => Ok(Self::Fold),
            "index" => Ok(Self::Index),
            "index_mut" => Ok(Self::IndexMut),
            "fn_once" => Ok(Self::FnOnce),
            "fn_mut" => Ok(Self::FnMut),
            "fn" => Ok(Self::Fn),
            "send" => Ok(Self::Send),
            "sync" => Ok(Self::Sync),
            "try" => Ok(Self::Try),
            "from_residual" => Ok(Self::FromResidual),
            "control_flow" => Ok(Self::ControlFlow),
            "range" => Ok(Self::Range),
            "method" => Ok(Self::Method),
            "output" => Ok(Self::Output),
            "residual" => Ok(Self::Residual),
            "branch" => Ok(Self::Branch),
            "break" => Ok(Self::Break),
            "continue" => Ok(Self::Continue),
            "range_full" => Ok(Self::RangeFull),
            "range_from" => Ok(Self::RangeFrom),
            "range_to" => Ok(Self::RangeTo),
            "range_to_inclusive" => Ok(Self::RangeToInclusive),
            "range_exclusive" => Ok(Self::RangeExclusive),
            "range_inclusive" => Ok(Self::RangeInclusive),
            _ => Err(format!(
                "unknown language item role '{role}'; expected one of: {}",
                Self::ALL_NAMES.join(", ")
            )),
        }
    }
}

impl fmt::Display for LanguageItemRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizedLanguageItems<D> {
    pub trait_id: D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropLanguageItems<D> {
    pub trait_id: D,
    pub method_id: D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldLanguageItems<D> {
    pub trait_id: D,
    pub method_id: D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexLanguageItems<D> {
    pub trait_id: D,
    pub output_id: AssocTypeId,
    pub method_id: D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexMutLanguageItems<D> {
    pub trait_id: D,
    pub output_id: AssocTypeId,
    pub method_id: D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FnOnceLanguageItems<D> {
    pub trait_id: D,
    pub output_id: AssocTypeId,
    pub method_id: D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FnMutLanguageItems<D> {
    pub trait_id: D,
    pub output_id: AssocTypeId,
    pub method_id: D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FnLanguageItems<D> {
    pub trait_id: D,
    pub output_id: AssocTypeId,
    pub method_id: D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendLanguageItems<D> {
    pub trait_id: D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncLanguageItems<D> {
    pub trait_id: D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TryLanguageItems<D> {
    pub try_trait_id: D,
    pub output_id: AssocTypeId,
    pub residual_id: AssocTypeId,
    pub branch_method_id: D,
    pub from_residual_trait_id: D,
    pub from_residual_method_id: D,
    pub control_flow_enum_id: D,
    pub break_variant_id: VariantId,
    pub continue_variant_id: VariantId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeLanguageItems<D> {
    pub enum_id: D,
    pub full_variant_id: VariantId,
    pub from_variant_id: VariantId,
    pub to_variant_id: VariantId,
    pub to_inclusive_variant_id: VariantId,
    pub exclusive_variant_id: VariantId,
    pub inclusive_variant_id: VariantId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageItems<D> {
    pub sized: Option<SizedLanguageItems<D>>,
    pub drop: Option<DropLanguageItems<D>>,
    pub fold: Option<FoldLanguageItems<D>>,
    pub index: Option<IndexLanguageItems<D>>,
    pub index_mut: Option<IndexMutLanguageItems<D>>,
    pub fn_once: Option<FnOnceLanguageItems<D>>,
    pub fn_mut: Option<FnMutLanguageItems<D>>,
    pub fn_trait: Option<FnLanguageItems<D>>,
    pub send: Option<SendLanguageItems<D>>,
    pub sync: Option<SyncLanguageItems<D>>,
    pub try_protocol: Option<TryLanguageItems<D>>,
    pub range: Option<RangeLanguageItems<D>>,
}

impl<D> Default for LanguageItems<D> {
    fn default() -> Self {
        Self {
            sized: None,
            drop: None,
            fold: None,
            index: None,
            index_mut: None,
            fn_once: None,
            fn_mut: None,
            fn_trait: None,
            send: None,
            sync: None,
            try_protocol: None,
            range: None,
        }
    }
}

impl<D: Copy> LanguageItems<D> {
    /// Invocation preference: shared, mutable, then consuming.
    pub fn callable_protocols(&self) -> impl Iterator<Item = (crate::types::CallableKind, D, D)> {
        use crate::types::CallableKind;
        [
            self.fn_trait
                .as_ref()
                .map(|item| (CallableKind::Fn, item.trait_id, item.method_id)),
            self.fn_mut
                .as_ref()
                .map(|item| (CallableKind::FnMut, item.trait_id, item.method_id)),
            self.fn_once
                .as_ref()
                .map(|item| (CallableKind::FnOnce, item.trait_id, item.method_id)),
        ]
        .into_iter()
        .flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageItemProviderConflict {
    Multiple {
        protocol: &'static str,
        providers: Vec<String>,
    },
    MissingRequired {
        provider: String,
        protocol: &'static str,
        required: &'static str,
    },
    SplitPair {
        left_protocol: &'static str,
        left_provider: String,
        right_protocol: &'static str,
        right_provider: String,
    },
}

impl fmt::Display for LanguageItemProviderConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Multiple {
                protocol,
                providers,
            } => write!(
                f,
                "multiple language-item providers claim protocol '{}': {}",
                protocol,
                providers.join(", "),
            ),
            Self::MissingRequired {
                provider,
                protocol,
                required,
            } => write!(
                f,
                "language-item provider '{provider}' claims protocol '{protocol}' without required protocol '{required}'",
            ),
            Self::SplitPair {
                left_protocol,
                left_provider,
                right_protocol,
                right_provider,
            } => write!(
                f,
                "language-item protocols '{left_protocol}' and '{right_protocol}' must use one provider; found {left_protocol}={left_provider}, {right_protocol}={right_provider}",
            ),
        }
    }
}

impl std::error::Error for LanguageItemProviderConflict {}

pub fn merge_language_item_providers<'a, D: Clone + 'a>(
    providers: impl IntoIterator<Item = (&'a str, &'a LanguageItems<D>)>,
) -> Result<LanguageItems<D>, LanguageItemProviderConflict> {
    merge_language_item_providers_all(providers).map_err(|mut conflicts| conflicts.remove(0))
}

pub fn merge_language_item_providers_all<'a, D: Clone + 'a>(
    providers: impl IntoIterator<Item = (&'a str, &'a LanguageItems<D>)>,
) -> Result<LanguageItems<D>, Vec<LanguageItemProviderConflict>> {
    let mut providers = providers.into_iter().collect::<Vec<_>>();
    providers.sort_by_key(|(name, _)| *name);

    let mut conflicts = Vec::new();
    let sized = merge_protocol(
        &providers,
        "sized",
        |items| items.sized.as_ref(),
        &mut conflicts,
    );
    let drop = merge_protocol(
        &providers,
        "drop",
        |items| items.drop.as_ref(),
        &mut conflicts,
    );
    let index_claims = providers
        .iter()
        .filter_map(|(name, items)| items.index.as_ref().map(|items| (*name, items)))
        .collect::<Vec<_>>();
    let fold = merge_protocol(
        &providers,
        "fold",
        |items| items.fold.as_ref(),
        &mut conflicts,
    );
    let index_mut_claims = providers
        .iter()
        .filter_map(|(name, items)| items.index_mut.as_ref().map(|items| (*name, items)))
        .collect::<Vec<_>>();

    match (index_claims.as_slice(), index_mut_claims.as_slice()) {
        ([], [(provider, _)]) => conflicts.push(LanguageItemProviderConflict::MissingRequired {
            provider: (*provider).to_string(),
            protocol: "index_mut",
            required: "index",
        }),
        ([(index_provider, _)], [(index_mut_provider, _)])
            if index_provider != index_mut_provider =>
        {
            conflicts.push(LanguageItemProviderConflict::SplitPair {
                left_protocol: "index",
                left_provider: (*index_provider).to_string(),
                right_protocol: "index_mut",
                right_provider: (*index_mut_provider).to_string(),
            });
        }
        _ => {}
    }

    let index = merge_claims("index", &index_claims, &mut conflicts);
    let index_mut = merge_claims("index_mut", &index_mut_claims, &mut conflicts);
    let fn_once = merge_protocol(
        &providers,
        "fn_once",
        |items| items.fn_once.as_ref(),
        &mut conflicts,
    );
    let fn_mut = merge_protocol(
        &providers,
        "fn_mut",
        |items| items.fn_mut.as_ref(),
        &mut conflicts,
    );
    let fn_trait = merge_protocol(
        &providers,
        "fn",
        |items| items.fn_trait.as_ref(),
        &mut conflicts,
    );
    let send = merge_protocol(
        &providers,
        "send",
        |items| items.send.as_ref(),
        &mut conflicts,
    );
    let sync = merge_protocol(
        &providers,
        "sync",
        |items| items.sync.as_ref(),
        &mut conflicts,
    );
    let try_protocol = merge_protocol(
        &providers,
        "try",
        |items| items.try_protocol.as_ref(),
        &mut conflicts,
    );
    let range = merge_protocol(
        &providers,
        "range",
        |items| items.range.as_ref(),
        &mut conflicts,
    );

    if conflicts.is_empty() {
        Ok(LanguageItems {
            sized,
            drop,
            fold,
            index,
            index_mut,
            fn_once,
            fn_mut,
            fn_trait,
            send,
            sync,
            try_protocol,
            range,
        })
    } else {
        Err(conflicts)
    }
}

fn merge_protocol<'a, D, T: Clone>(
    providers: &[(&'a str, &'a LanguageItems<D>)],
    protocol: &'static str,
    bundle: impl Fn(&LanguageItems<D>) -> Option<&T>,
    conflicts: &mut Vec<LanguageItemProviderConflict>,
) -> Option<T> {
    let claims = providers
        .iter()
        .filter_map(|(name, items)| bundle(items).map(|items| (*name, items)))
        .collect::<Vec<_>>();

    merge_claims(protocol, &claims, conflicts)
}

fn merge_claims<T: Clone>(
    protocol: &'static str,
    claims: &[(&str, &T)],
    conflicts: &mut Vec<LanguageItemProviderConflict>,
) -> Option<T> {
    match claims {
        [] => None,
        [(_, items)] => Some((*items).clone()),
        _ => {
            conflicts.push(LanguageItemProviderConflict::Multiple {
                protocol,
                providers: claims.iter().map(|(name, _)| name.to_string()).collect(),
            });
            None
        }
    }
}

#[cfg(test)]
mod tests;
