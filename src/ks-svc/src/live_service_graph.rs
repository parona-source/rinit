use std::{
    collections::HashMap,
    io,
    path::Path,
    process::Stdio,
    sync::Arc,
};

use anyhow::Result;
use async_recursion::async_recursion;
use kansei_core::{
    graph::DependencyGraph,
    types::Service,
};
use kansei_message::Message;
use tokio::{
    fs::{
        self,
        File,
    },
    io::AsyncWriteExt,
    net::UnixListener,
    process::Command,
    sync::RwLock,
};

use crate::{
    live_service::{
        LiveService,
        ServiceStatus,
    },
    CONFIG,
};

pub struct LiveServiceGraph {
    indexes: HashMap<String, usize>,
    live_services: RwLock<Vec<Arc<RwLock<LiveService>>>>,
}

impl LiveServiceGraph {
    pub fn new(graph: DependencyGraph) -> Result<Self> {
        let nodes = graph
            .nodes
            .into_iter()
            .map(LiveService::new)
            .collect::<Vec<_>>();
        Ok(Self {
            indexes: nodes
                .iter()
                .enumerate()
                .map(|(i, el)| (el.node.name().to_owned(), i))
                .collect(),
            live_services: RwLock::new(
                nodes
                    .into_iter()
                    .map(|node| Arc::new(RwLock::new(node)))
                    .collect(),
            ),
        })
    }

    pub async fn start_all_services(&'static self) {
        let services = self.live_services.read().await;
        let futures: Vec<_> = services
            .clone()
            .into_iter()
            .map(|live_service| {
                tokio::spawn(async move {
                    let should_start = {
                        let live_service = live_service.read().await;
                        live_service.node.service.should_start()
                    };
                    if should_start {
                        self.start_service_impl(live_service.clone()).await;
                    }
                })
            })
            .collect();
        for future in futures {
            future.await.unwrap();
        }
    }

    #[async_recursion]
    async fn start_service(
        &self,
        live_service: Arc<RwLock<LiveService>>,
    ) {
        {
            live_service
                .write()
                .await
                .change_status(ServiceStatus::Starting)
                .await;
        }
        self.start_dependencies(&live_service).await;
        self.start_service_impl(live_service).await;
    }

    async fn start_dependencies(
        &self,
        live_service: &Arc<RwLock<LiveService>>,
    ) {
        let dependencies = {
            let live_service = live_service.read().await;
            live_service.node.service.dependencies().to_owned()
        };
        // Start dependencies
        let futures: Vec<_> = dependencies
            .iter()
            .map(async move |dep| {
                let services = self.live_services.read().await;
                let dep_service = services
                    .get(*self.indexes.get(dep).expect("This should nevel happen"))
                    .unwrap();
                let res = {
                    let lock = dep_service.read().await;
                    let status = lock.status.lock().await;
                    *status != ServiceStatus::Up
                        && *status != ServiceStatus::Starting
                        && *status != ServiceStatus::Stopping
                };
                if res {
                    self.start_service(dep_service.clone()).await;
                }
            })
            .collect();
        for future in futures {
            future.await;
        }
    }

    async fn start_service_impl(
        &self,
        live_service: Arc<RwLock<LiveService>>,
    ) -> Result<()> {
        self.wait_on_deps(live_service.clone()).await;
        let live_service = live_service.read().await;
        let res = match &live_service.node.service {
            Service::Oneshot(oneshot) => Some(("ks-run-oneshot", serde_json::to_vec(&oneshot))),
            Service::Longrun(longrun) => Some(("ks-run-longrun", serde_json::to_vec(&longrun))),
            Service::Bundle(_) => None,
            Service::Virtual(_) => None,
        };
        if let Some((exe, ser_res)) = res {
            let config = CONFIG.read().await;
            let runtime_service_dir = config
                .as_ref()
                .rundir
                .as_ref()
                .unwrap()
                .join(&live_service.node.name());
            fs::create_dir_all(&runtime_service_dir).await.unwrap();
            let service_path = runtime_service_dir.join("service");
            let mut file = File::create(service_path).await.unwrap();
            let buf = ser_res.unwrap();
            file.write(&buf).await.unwrap();
            // TODO: Add logging and remove unwrap
            Command::new(exe)
                .args(vec![runtime_service_dir])
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .spawn()
                .unwrap();
        }

        Ok(())
    }

    async fn get_status(
        &self,
        name: &str,
    ) -> ServiceStatus {
        let services = self.live_services.read().await;
        let dep_service_rw = services
            .get(*self.indexes.get(name).expect("This should never happen"))
            .unwrap();
        let status = {
            let dep_service = dep_service_rw.read().await;
            dep_service.get_status().await
        };
        if let Some(status) = status {
            status
        } else {
            let notifier = {
                let dep_service = dep_service_rw.read().await;
                dep_service.wait_on_status()
            };
            notifier.notified().await;
            let dep_service = dep_service_rw.read().await;
            dep_service.get_status().await;
            status.unwrap()
        }
    }

    async fn wait_on_deps(
        &self,
        live_service: Arc<RwLock<LiveService>>,
    ) {
        let dependencies = {
            let live_service = live_service.read().await;
            live_service.node.service.dependencies().to_owned()
        };
        let futures: Vec<_> = dependencies
            .iter()
            .map(async move |dep| -> ServiceStatus { self.get_status(dep).await })
            .collect();

        for future in futures {
            future.await;
        }
    }
}
