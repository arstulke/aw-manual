//! Core library of aw-manual: configuration and favorites, time parsing,
//! the tracking state machine, the offline queue, the ActivityWatch client
//! and the IPC protocol.

pub mod afk;
pub mod aw;
pub mod config;
pub mod display;
pub mod duration;
pub mod engine;
pub mod favorites;
pub mod format;
pub mod ipc;
pub mod model;
pub mod paths;
pub mod reminders;
pub mod service;
pub mod store;
pub mod timeparse;
pub mod writer;
pub mod yaml;
pub mod yaml_edit;
