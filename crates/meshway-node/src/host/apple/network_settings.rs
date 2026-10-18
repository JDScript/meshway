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
            let bytes = serde_json::to_vec(&CaptureRecord {
                version: 1,
                settings: settings.clone(),
            })
            .map_err(io::Error::other)?;
            self.directory
                .replace(JOURNAL, &bytes)
                .map_err(io::Error::other)?;
        }
        self.sink.apply(next.clone()).await?;
        if next.is_none() {
            self.directory.remove(JOURNAL).map_err(io::Error::other)?;
        }
        self.current = next;
        self.failed = false;
        Ok(())
    }
}
