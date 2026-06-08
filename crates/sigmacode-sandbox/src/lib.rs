use std::path::Path;

#[derive(Debug, Clone)]
pub enum SandboxPolicy {
    None,
    DiskRead,
    DiskWriteTemp,
    NetworkRestricted,
    FullIsolation,
}

pub struct Sandbox;

impl Sandbox {
    pub fn new(_policy: SandboxPolicy, _workspace: &Path) -> Self {
        Self
    }
}
