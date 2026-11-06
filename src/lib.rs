//! LawGit core library: legal documents and rule documents as XML in Git.

pub mod api;
pub mod check;
pub mod cli;
pub mod git;
pub mod json;
pub mod law;
pub mod repo;
pub mod rpc;
pub mod rule;
pub mod server;
pub mod util;
pub mod uuid;
pub mod xml;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
