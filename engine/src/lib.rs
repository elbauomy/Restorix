//! Restorix recovery engine.
//!
//! Every source is opened read-only. Recovered data is only ever written to a destination folder
//! chosen by the user, which must be on a different physical disk for raw (metadata/carved) items.

pub mod carve;
pub mod devices;
pub mod fs;
pub mod io;
pub mod model;
pub mod partition;
pub mod preview;
pub mod query;
pub mod restore;
pub mod scan;
pub mod snapshots;
pub mod trash;
pub mod util;

pub use model::{FileItem, ScanCtx};
pub use scan::ScanRequest;
