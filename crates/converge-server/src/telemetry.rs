//! Tracing setup from `[log]` config.

use anyhow::Context;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

use crate::config::Log;

/// Initialize the global subscriber. Log writes go through a dedicated
/// thread (bounded channel, lossy under sustained backpressure) so request
/// tasks never block on stdout. The returned guard flushes the buffer on
/// drop — hold it for the life of `main`.
pub fn init(log: &Log) -> anyhow::Result<WorkerGuard> {
    let (stdout, guard) = tracing_appender::non_blocking(std::io::stdout());
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(&log.filter).context("log.filter")?)
        .with_writer(stdout)
        .init();
    Ok(guard)
}
