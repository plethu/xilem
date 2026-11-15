// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

//! Helpers for testing views against Masonry's test harness.

use std::fmt::Debug;
use std::sync::Arc;

use crate::ViewCtx;
use crate::core::{ProxyError, RawProxy, SendMessage, ViewId};

#[derive(Debug)]
struct NoProxy;

impl RawProxy for NoProxy {
    fn send_message(&self, _path: Arc<[ViewId]>, _message: SendMessage) -> Result<(), ProxyError> {
        Ok(())
    }

    fn dyn_debug(&self) -> &dyn Debug {
        self
    }
}

/// A view context whose proxy drops every message.
pub(crate) fn view_ctx() -> ViewCtx {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build a test runtime");
    ViewCtx::new(Arc::new(NoProxy), Arc::new(runtime))
}
