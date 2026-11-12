//! Btw-aangifte, ICP-opgave and suppletie: drawing up, reading and correcting. Never filing.
//!
//! The tools of this part live here, on their own router (`btw_aangifte_router`), which tools.rs adds to the
//! server's. Each tool is named in one of the lists below, which the tool-list test in mcp.rs reads.

use super::*;

/// The tools of this part that only read.
#[cfg(test)]
pub(super) const READS: &[&str] = &[];
/// The tools of this part that change the books.
#[cfg(test)]
pub(super) const WRITES: &[&str] = &[];
/// The ones in `WRITES` that cannot be undone.
#[cfg(test)]
pub(super) const DESTRUCTIVE: &[&str] = &[];

#[tool_router(router = btw_aangifte_router, vis = "pub(super)", allow_empty)]
impl EbMcp {}
