use std::sync::Arc;
use tokio::sync::mpsc as _;
use crate::gateway::IncomingMessage;

pub enum PipelineAction {
    Allow,
    Block(String),
    Queue,
}

pub trait MessageFilter: Send + Sync {
    fn check(&self, msg: &IncomingMessage) -> PipelineAction;
}

pub struct MessagePipeline {
    filters: Vec<Box<dyn MessageFilter>>,
}

impl MessagePipeline {
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
        }
    }

    pub fn add_filter(&mut self, filter: Box<dyn MessageFilter>) {
        self.filters.push(filter);
    }

    pub fn process(&self, msg: &IncomingMessage) -> PipelineAction {
        for filter in &self.filters {
            match filter.check(msg) {
                PipelineAction::Allow => continue,
                action => return action,
            }
        }
        PipelineAction::Allow
    }
}
