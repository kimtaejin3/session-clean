//! sclean 코어 라이브러리.
//!
//! TUI(`ui`)와 순수 코어(`scan`, `rules`, `ops`)는 분리되어 있고,
//! 코어는 터미널 없이 테스트할 수 있다.

pub mod config;
pub mod live;
pub mod logging;
pub mod ops;
pub mod paths;
pub mod rules;
pub mod scan;
pub mod ui;

pub use paths::Paths;
pub use scan::session::{Project, ScanResult, Session, SessionKind};
