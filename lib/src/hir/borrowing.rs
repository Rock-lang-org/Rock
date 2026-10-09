//! Portable interface borrow authority; paths and argument ordinals are not MIR locals.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BorrowProjection {
    Field(u32),
    Element,
    Deref,
}

pub type BorrowPath = Vec<BorrowProjection>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BorrowSourceKind {
    /// References already carried by the argument value (including opaque captures).
    Value,
    /// References carried inside borrowed storage, not a loan of that storage.
    Contents,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BorrowSource {
    pub argument: u32,
    pub path: BorrowPath,
    pub kind: BorrowSourceKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BorrowReturn {
    pub path: BorrowPath,
    pub sources: Vec<BorrowSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BorrowRetention {
    pub destination: BorrowSource,
    pub sources: Vec<BorrowSource>,
}

/// Conservative declaration authority, checked against bodies and instantiated at calls.
/// Static references need no caller loan and are always authorized; local storage never is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BorrowContract {
    pub returns: Vec<BorrowReturn>,
    pub retentions: Vec<BorrowRetention>,
}

/// Producer-checked return dependencies for one closed native signature.
/// No mutation/effect knowledge and no producer-local identities are carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NativeReturnSource {
    ArgumentValue(u32),
    ArgumentContents(u32),
    ArgumentBorrowedContents(u32),
    Static,
    /// An opaque external dependency measured in a checked scalar producer.
    /// This is neither independence nor mutation/retention authority.
    UnknownExternal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeReturnOrigins<T> {
    pub params: Vec<T>,
    pub ret: T,
    pub sources: Vec<NativeReturnSource>,
}

impl<T> NativeReturnOrigins<T> {
    pub fn map_types<U>(
        &self,
        map: &mut impl FnMut(&T) -> Result<U, String>,
    ) -> Result<NativeReturnOrigins<U>, String> {
        Ok(NativeReturnOrigins {
            params: self
                .params
                .iter()
                .map(&mut *map)
                .collect::<Result<_, _>>()?,
            ret: map(&self.ret)?,
            sources: self.sources.clone(),
        })
    }

    pub fn sources_are_valid(&self, contents_type: impl Fn(&T) -> bool) -> bool {
        if self.sources.is_empty() || self.sources.windows(2).any(|pair| pair[0] >= pair[1]) {
            return false;
        }
        self.sources.iter().all(|source| match source {
            NativeReturnSource::ArgumentValue(index) => self.params.get(*index as usize).is_some(),
            NativeReturnSource::ArgumentContents(index)
            | NativeReturnSource::ArgumentBorrowedContents(index) => {
                self.params.get(*index as usize).is_some_and(&contents_type)
            }
            NativeReturnSource::Static | NativeReturnSource::UnknownExternal => true,
        })
    }
}
