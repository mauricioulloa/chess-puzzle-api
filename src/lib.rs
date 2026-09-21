//! Open-source chess puzzle API backed by the Lichess puzzle database (CC0).
//!
//! The crate is split into a library and a thin CLI binary so the importer and
//! the query layer can be exercised directly from integration tests.

pub mod api;
pub mod auth;
pub mod chess;
pub mod db;
pub mod import;
pub mod serve;
