//! Which session the interface is driving: one it started itself, or one already
//! running that it has attached to. The interface asks for the same things either way.

use std::path::Path;

use anyhow::Result;

use crate::config::Settings;
use crate::engine::{DownloadId, Engine, Input, Progress};
use crate::server::Remote;

pub enum Session {
    Local(Engine),
    Attached(Remote),
}

impl Session {
    /// The address of the session this is driving, when that is somewhere else.
    pub fn elsewhere(&self) -> Option<&str> {
        match self {
            Self::Local(_) => None,
            Self::Attached(remote) => Some(remote.address()),
        }
    }

    pub async fn add(&self, input: &Input, folder: &Path) -> Result<()> {
        match self {
            Self::Local(engine) => engine.add(input, folder).await.map(drop),
            Self::Attached(remote) => remote.add(input, folder).await,
        }
    }

    pub async fn snapshot(&self) -> Result<Vec<Progress>> {
        match self {
            Self::Local(engine) => Ok(engine.snapshot()),
            Self::Attached(remote) => remote.snapshot().await,
        }
    }

    pub async fn pause(&self, id: DownloadId) -> Result<()> {
        match self {
            Self::Local(engine) => engine.pause(id).await,
            Self::Attached(remote) => remote.pause(id).await,
        }
    }

    pub async fn resume(&self, id: DownloadId) -> Result<()> {
        match self {
            Self::Local(engine) => engine.resume(id).await,
            Self::Attached(remote) => remote.resume(id).await,
        }
    }

    pub async fn remove(&self, id: DownloadId) -> Result<()> {
        match self {
            Self::Local(engine) => engine.remove(id).await,
            Self::Attached(remote) => remote.remove(id).await,
        }
    }

    /// The queue belongs to whoever owns the session. A daemon runs it on its own
    /// timer, against its own settings, and a second opinion from here would be a
    /// fight rather than a queue.
    pub async fn enforce(&self, settings: &Settings) -> Result<()> {
        match self {
            Self::Local(engine) => engine.enforce(settings).await,
            Self::Attached(_) => Ok(()),
        }
    }

    pub fn apply(&self, settings: &Settings) {
        if let Self::Local(engine) = self {
            engine.apply(settings);
        }
    }

    /// Quitting an attached interface leaves the session it was driving running. That
    /// is the whole point of having attached to it.
    pub async fn shutdown(&self) {
        if let Self::Local(engine) = self {
            engine.shutdown().await;
        }
    }
}
