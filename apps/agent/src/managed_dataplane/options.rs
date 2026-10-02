use crate::AgentError;
#[derive(Clone, Debug)]
pub struct TunOptions {
    pub name: String,
    pub mtu: u16,
}
impl TunOptions {
    pub(crate) fn validate(&self) -> Result<(), AgentError> {
        if self.name.is_empty()
            || self.name.len() > 15
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !(576..=1400).contains(&self.mtu)
        {
            return Err(AgentError::Invalid(
                "invalid TUN name or IPv4 MTU (576–1400)",
            ));
        }
        Ok(())
    }
}
