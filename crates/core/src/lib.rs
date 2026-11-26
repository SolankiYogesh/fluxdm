//! FluxDM download engine.

pub mod events;
pub mod fsio;
pub mod ftp;
pub mod hls;
pub mod http;
pub mod ipc;
pub mod limiter;
pub mod links;
pub mod manager;
pub mod naming;
pub mod preview;
pub mod segments;
pub mod types;

pub use events::Event;
pub use manager::{Manager, TaskView};
pub use types::{DownloadRequest, Schedule, Settings, Status, Task};
