use std::collections::HashSet;

use async_trait::async_trait;

use crate::error::Result;
use crate::plugin::{InterceptedRequest, Plugin, RequestDecision};

/// Blocks configured request resource types (e.g. image, stylesheet, media).
#[derive(Debug, Clone, Default)]
pub struct BlockResourcesPlugin {
    blocked_types: HashSet<String>,
}

impl BlockResourcesPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn block_type(mut self, resource_type: impl Into<String>) -> Self {
        self.blocked_types.insert(resource_type.into());
        self
    }
}

#[async_trait]
impl Plugin for BlockResourcesPlugin {
    fn name(&self) -> &'static str {
        "block-resources"
    }

    fn priority(&self) -> i32 {
        10
    }

    async fn on_request(&self, request: &InterceptedRequest) -> Result<RequestDecision> {
        let Some(resource_type) = &request.resource_type else {
            return Ok(RequestDecision::Continue);
        };
        if self.blocked_types.contains(resource_type) {
            Ok(RequestDecision::Abort)
        } else {
            Ok(RequestDecision::Continue)
        }
    }
}
