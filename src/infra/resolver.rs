use std::collections::HashMap;
use std::net::IpAddr;

use crate::domain::traffic::{EntityId, Resolver, Socket};
use crate::domain::Scope;
use crate::infra::engine::{ContainerInfo, Engine};

pub struct RuntimeResolver {
    ip_map: HashMap<IpAddr, EntityId>,
}

impl RuntimeResolver {
    pub fn from_engine(
        engine: &Engine,
        run_id: &str,
        service_aliases: &HashMap<String, String>,
    ) -> Self {
        let ids = engine.collect_run_container_ids(run_id, Scope::Running);
        let containers = engine.inspect_containers(&ids);
        Self {
            ip_map: build_ip_map(containers, service_aliases),
        }
    }

    pub fn resolve_ip(&self, ip: &IpAddr) -> Option<EntityId> {
        self.ip_map.get(ip).cloned()
    }
}

impl Resolver for RuntimeResolver {
    fn resolve_entity(&self, socket: &Socket) -> Option<EntityId> {
        self.resolve_ip(&socket.ip)
    }
}

fn build_ip_map(
    containers: Vec<ContainerInfo>,
    service_aliases: &HashMap<String, String>,
) -> HashMap<IpAddr, EntityId> {
    let mut map = HashMap::new();
    for container in containers {
        let name = container
            .service
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let name = service_aliases.get(&name).cloned().unwrap_or(name);
        let instance = if container.id.is_empty() {
            None
        } else {
            Some(container.id.chars().take(12).collect())
        };
        let entity = EntityId::Workload { name, instance };
        for ip in container.ips {
            map.insert(ip, entity.clone());
        }
    }
    map
}
