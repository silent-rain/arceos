#[cfg(feature = "alloc")]
extern crate alloc;

#[allow(unused_imports)]
#[cfg(feature = "alloc")]
use alloc::collections::*;

#[cfg(feature = "hash_map")]
pub mod hash_map;
#[cfg(feature = "hash_map")]
pub use hash_map::HashMap;

pub mod random;
