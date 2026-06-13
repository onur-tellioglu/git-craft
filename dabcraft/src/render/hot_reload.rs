use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

/// Watches several named shader files; `poll()` returns every (name, source)
/// that changed and validated since the last call. Each inner watcher keeps
/// its own 500 ms poll throttle.
#[derive(Default)]
pub struct ShaderSet {
    watchers: Vec<(&'static str, ShaderWatcher)>,
}

impl ShaderSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn watch(&mut self, name: &'static str, path: impl Into<PathBuf>) {
        self.watchers.push((name, ShaderWatcher::new(path)));
    }

    pub fn poll(&mut self) -> Vec<(&'static str, String)> {
        self.watchers
            .iter_mut()
            .filter_map(|(name, w)| w.poll().map(|src| (*name, src)))
            .collect()
    }
}

pub fn validate_wgsl(source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.to_string())?;
    // Baseline capabilities only: a shader that needs optional capabilities
    // would pass an all() check here yet still fail wgpu's own (stricter)
    // validation at pipeline creation, defeating this pre-swap gate.
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
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
        // Read before committing the mtime: editors that atomically rename or
        // truncate-then-write can make this read fail or see partial content;
        // leaving last_mtime untouched lets the next poll retry that save.
        let source = std::fs::read_to_string(&self.path).ok()?;
        self.last_mtime = Some(mtime);
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
    fn all_shipped_shaders_are_valid() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/shaders");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "wgsl") {
                let src = std::fs::read_to_string(&path).unwrap();
                if let Err(e) = validate_wgsl(&src) {
                    panic!("{} failed validation:\n{e}", path.display());
                }
                checked += 1;
            }
        }
        // All M5a shaders: terrain, outline, post, shadow, sky_luts, sky,
        // bloom, exposure. Catches an accidentally deleted file.
        assert!(
            checked >= 8,
            "expected >= 8 shaders (terrain, outline, post, shadow, sky_luts, sky, bloom, exposure), found {checked}"
        );
    }
}
