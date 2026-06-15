pub mod block;
pub mod chunks;
pub mod decor;
pub mod r#gen;
pub mod jobs;
pub mod light;
pub mod light_engine;
pub mod persistence;
pub mod region;
pub mod section;

/// Take a fixed-size chunk from `bytes` at byte cursor `c`, advancing `c` by `N`.
/// Returns `None` if the slice would exceed the buffer (truncation guard).
/// Used by `region` and `section` for little-endian deserialization.
pub(super) fn take<const N: usize>(bytes: &[u8], c: &mut usize) -> Option<[u8; N]> {
    let end = c.checked_add(N)?;
    let slice = bytes.get(*c..end)?;
    *c = end;
    Some(slice.try_into().expect("slice length checked above"))
}
