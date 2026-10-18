//! One serialized owner for NE capture and DNS settings and their journal.
use super::*;

pub(super) struct Owner {
    pub directory: Arc<meshway_private_storage::Directory>,
    pub sink: Arc<dyn TunnelSettingsSink>,
    pub current: Option<TunnelSettings>,
    pub pools: Vec<ipnet::IpNet>,
    pub failed: bool,
}
impl Owner {
    pub async fn apply(&mut self, next: Option<TunnelSettings>) -> io::Result<()> {
        if !self.failed && self.current == next {
            return Ok(());
        }
        self.failed = true;
        if let Some(settings) = &next {
            // A failed or interrupted NE replacement must not forget an old
            // negative boundary at restart. Stage the union before shrinking.
            let mut retained = settings.clone();
            if let Some(previous) = &self.current {
                retained.routes.extend(&previous.routes);
                retained.routes.sort();
                retained.routes.dedup();
                retained.egress_routes.extend(&previous.egress_routes);
                retained.egress_routes = ipnet::IpNet::aggregate(&retained.egress_routes);
            }
            if retained.routes.len() > 128 || retained.egress_routes.len() > capture::MAX_ROUTES {
                return Err(io::Error::other("capture transition prefix capacity"));
            }
            let bytes = serde_json::to_vec(&CaptureRecord {
                version: 1,
                settings: retained.clone(),
            })
            .map_err(io::Error::other)?;
            self.directory
                .replace(JOURNAL, &bytes)
                .map_err(io::Error::other)?;
            self.current = Some(retained.clone());
            if retained != *settings {
                self.sink.apply(Some(retained)).await?;
            }
        }
        self.sink.apply(next.clone()).await?;
        if let Some(settings) = &next {
            let bytes = serde_json::to_vec(&CaptureRecord {
                version: 1,
                settings: settings.clone(),
            })
            .map_err(io::Error::other)?;
            self.directory
                .replace(JOURNAL, &bytes)
                .map_err(io::Error::other)?;
        } else {
            self.directory.remove(JOURNAL).map_err(io::Error::other)?;
        }
        self.current = next;
        self.failed = false;
        Ok(())
    }
}
