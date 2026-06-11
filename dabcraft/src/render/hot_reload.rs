use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

pub fn validate_wgsl(source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.to_string())?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub struct ShaderWatcher {
    path: PathBuf,
    last_mtime: Option<SystemTime>,
    last_check: Instant,
}

impl ShaderWatcher {
    const POLL_INTERVAL: Duration = Duration::from_millis(500);

    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let last_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        Self { path, last_mtime, last_check: Instant::now() }
    }

    /// Returns validated new shader source when the file changed and is valid.
    pub fn poll(&mut self) -> Option<String> {
        if self.last_check.elapsed() < Self::POLL_INTERVAL {
            return None;
        }
        self.last_check = Instant::now();
        let mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok()?;
        if Some(mtime) == self.last_mtime {
            return None;
        }
        self.last_mtime = Some(mtime);
        let source = std::fs::read_to_string(&self.path).ok()?;
        match validate_wgsl(&source) {
            Ok(()) => {
                log::info!("shader reloaded: {}", self.path.display());
                Some(source)
            }
            Err(e) => {
                log::error!("shader error (keeping previous pipeline):\n{e}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_wgsl_passes() {
        assert!(validate_wgsl("@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4(0.0); }").is_ok());
    }

    #[test]
    fn syntax_error_is_reported_not_panicked() {
        assert!(validate_wgsl("@vertex fn vs( {").is_err());
    }

    #[test]
    fn type_error_is_reported() {
        assert!(validate_wgsl("@vertex fn vs() -> @builtin(position) vec4<f32> { return 1; }").is_err());
    }

    #[test]
    fn shipped_terrain_shader_is_valid() {
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/shaders/terrain.wgsl")).unwrap();
        assert!(validate_wgsl(&src).is_ok());
    }
}
