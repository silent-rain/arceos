#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::collections::*;

pub mod hash_map;
pub use hash_map::HashMap;

pub mod random;

// pub use hash_map;
// use std::collections::HashMap;
