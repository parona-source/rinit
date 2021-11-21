use async_pidfd::PidFd;
use async_std::sync::{
    Condvar,
    Mutex,
};
use chrono::prelude::*;
use kansei_core::{
    graph::Node,
    types::ScriptConfig,
};

#[derive(Clone, PartialEq)]
pub enum ServiceStatus {
    Reset,
    Up,
    Down,
    Starting,
    Stopping,
}

pub struct LiveService {
    pub node: Node,
    pub updated_node: Option<Node>,
    pub status: Mutex<ServiceStatus>,
    pub status_changed: Option<DateTime<Local>>,
    pub wait: Condvar,
    // Skip starting and stopping values here
    pub last_status: Option<ServiceStatus>,
    // first element for Oneshot::start and Longrun::run
    // second element for Oneshot::stop and Longrun::finish
    pub config: Option<(ScriptConfig, ScriptConfig)>,
    pub environment: Option<(ScriptConfig, ScriptConfig)>,
    pub remove: bool,
    pub supervisor: Option<PidFd>,
}

impl LiveService {
    pub fn new(node: Node) -> Self {
        Self {
            node,
            updated_node: None,
            status: Mutex::new(ServiceStatus::Reset),
            status_changed: None,
            wait: Condvar::new(),
            last_status: None,
            config: None,
            environment: None,
            remove: false,
            supervisor: None,
        }
    }

    pub async fn change_status(
        &mut self,
        new_status: ServiceStatus,
    ) {
        let mut status = self.status.lock().await;
        match *status {
            ServiceStatus::Starting => {}
            ServiceStatus::Stopping => {}
            _ => {
                self.last_status = Some(status.clone());
            }
        }
        *status = new_status;
        self.status_changed = Some(chrono::offset::Local::now());
        self.wait.notify_all();
    }

    pub async fn wait_on_status(&self) -> ServiceStatus {
        (*self
            .wait
            .wait_until(self.status.lock().await, |status| {
                *status == ServiceStatus::Up || *status == ServiceStatus::Down
            })
            .await)
            .clone()
    }
}
