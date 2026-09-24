//! Device drivers: each one speaks a device's hardware protocol and nothing more. What is built on
//! top of them (the filesystem, the console, the keyboard's tokens and lines) lives in its own
//! module above this one.

pub mod virtio;
