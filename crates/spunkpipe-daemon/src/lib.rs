//! The background engine: polls the server, downloads updates, and applies them when the game
//! is closed. It has no UI of its own; the tray (or a headless runner) drives it.

pub mod autostart;
pub mod config;
pub mod daemon;
pub mod engine;
pub mod game;
pub mod paths;

pub use config::{Config, GameConfig, Mode};
pub use daemon::{Daemon, Detected};
pub use spunkpipe_profile::{FolderKind, ModFolder};
pub use engine::{Command, Engine, EngineConfig, Handle, Snapshot, Status};
pub use game::{GameProbe, NeverRunning, ProcessProbe};
