//! On-disk region files: persistence for player-edited columns.
//!
//! Worldgen is deterministic, so only edited columns are saved — an untouched
//! column regenerates identically. A region groups `REGION_COLS × REGION_COLS`
//! columns into one file (`r.<rx>.<rz>.gcr`), read-modify-written atomically.
//! Light is never stored: a loaded column recomputes it via the generation
//! path. The (de)serialization here is pure; the [`RegionStore`] filesystem ops
//! are tested against a temp dir.

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use crate::world::chunks::ColumnPos;
use crate::world::r#gen::COLUMN_SECTIONS;
use crate::world::section::Section;

const MAGIC: [u8; 4] = *b"GCR1";
const VERSION: u16 = 1;
/// 2^5 = 32 columns per region axis → 1024 columns per region file.
const REGION_SHIFT: i32 = 5;
const REGION_COLS: i32 = 1 << REGION_SHIFT;

/// Take a fixed-size chunk from `bytes` at cursor `c`, advancing it; None on truncation.
fn take<const N: usize>(bytes: &[u8], c: &mut usize) -> Option<[u8; N]> {
    let end = c.checked_add(N)?;
    let slice = bytes.get(*c..end)?;
    *c = end;
    Some(slice.try_into().expect("slice length checked above"))
}

/// Region grid coordinate containing `col` (floor division; handles negatives).
pub fn region_of(col: ColumnPos) -> (i32, i32) {
    (col.x >> REGION_SHIFT, col.z >> REGION_SHIFT)
}

/// Local slot (`0..1024`) of `col` within its region file.
pub fn local_index(col: ColumnPos) -> u16 {
    let lx = col.x.rem_euclid(REGION_COLS);
    let lz = col.z.rem_euclid(REGION_COLS);
    (lz * REGION_COLS + lx) as u16
}

/// Inverse of [`region_of`] + [`local_index`]: the column at `index` in `region`.
pub fn column_at(region: (i32, i32), index: u16) -> ColumnPos {
    let (rx, rz) = region;
    let lx = index as i32 % REGION_COLS;
    let lz = index as i32 / REGION_COLS;
    ColumnPos {
        x: rx * REGION_COLS + lx,
        z: rz * REGION_COLS + lz,
    }
}

/// Concatenate a column's sections (block data only) into one payload.
pub fn serialize_column(sections: &[Arc<Section>]) -> Vec<u8> {
    let mut out = Vec::new();
    for s in sections {
        s.write_bytes(&mut out);
    }
    out
}

/// Parse a payload written by [`serialize_column`] into exactly
/// `COLUMN_SECTIONS` sections. `None` on truncation, the wrong count, or
/// trailing bytes — a corrupt payload degrades to regeneration, not a panic.
pub fn deserialize_column(bytes: &[u8]) -> Option<Vec<Section>> {
    let mut sections = Vec::with_capacity(COLUMN_SECTIONS);
    let mut c = 0usize;
    for _ in 0..COLUMN_SECTIONS {
        let (s, n) = Section::read_bytes(bytes.get(c..)?)?;
        sections.push(s);
        c += n;
    }
    (c == bytes.len()).then_some(sections)
}

/// Serialize a region's present columns (local index → payload). Deterministic
/// order (BTreeMap) → byte-identical files for identical content.
pub fn serialize_region(columns: &BTreeMap<u16, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(columns.len() as u16).to_le_bytes());
    for (index, payload) in columns {
        out.extend_from_slice(&index.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
    }
    out
}

/// Parse a blob written by [`serialize_region`]. `None` on a bad magic or truncation.
pub fn parse_region(bytes: &[u8]) -> Option<BTreeMap<u16, Vec<u8>>> {
    let mut c = 0usize;
    if take::<4>(bytes, &mut c)? != MAGIC {
        return None;
    }
    let version = u16::from_le_bytes(take::<2>(bytes, &mut c)?);
    if version != VERSION {
        return None;
    }
    let count = u16::from_le_bytes(take::<2>(bytes, &mut c)?);
    let mut map = BTreeMap::new();
    for _ in 0..count {
        let index = u16::from_le_bytes(take::<2>(bytes, &mut c)?);
        let len = u32::from_le_bytes(take::<4>(bytes, &mut c)?) as usize;
        let end = c.checked_add(len)?;
        let payload = bytes.get(c..end)?.to_vec();
        c = end;
        map.insert(index, payload);
    }
    Some(map)
}

/// "r.<rx>.<rz>.gcr" → `(rx, rz)`; rejects temp files and anything else.
fn parse_region_filename(name: &str) -> Option<(i32, i32)> {
    let rest = name.strip_prefix("r.")?.strip_suffix(".gcr")?;
    let (rx, rz) = rest.split_once('.')?;
    Some((rx.parse().ok()?, rz.parse().ok()?))
}

/// A directory of region files. All methods are independent of any worker
/// thread; [`crate::world::persistence`] funnels every call through one thread
/// so a region file is never written concurrently.
pub struct RegionStore {
    dir: PathBuf,
}

impl RegionStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn region_path(&self, rx: i32, rz: i32) -> PathBuf {
        self.dir.join(format!("r.{rx}.{rz}.gcr"))
    }

    /// Load and parse a region file. `Ok(None)` if it does not exist or is
    /// corrupt (treated as absent so the columns simply regenerate).
    fn load_region_map(&self, rx: i32, rz: i32) -> io::Result<Option<BTreeMap<u16, Vec<u8>>>> {
        match std::fs::read(self.region_path(rx, rz)) {
            Ok(bytes) => Ok(parse_region(&bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The stored payload for `col`, or `Ok(None)` if it was never saved.
    pub fn load_column(&self, col: ColumnPos) -> io::Result<Option<Vec<u8>>> {
        let (rx, rz) = region_of(col);
        Ok(self
            .load_region_map(rx, rz)?
            .and_then(|m| m.get(&local_index(col)).cloned()))
    }

    /// Insert/replace `col`'s payload in its region file (read-modify-write via
    /// a temp file + atomic rename, so a crash mid-write can't truncate it).
    pub fn save_column(&self, col: ColumnPos, payload: Vec<u8>) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let (rx, rz) = region_of(col);
        let mut map = self.load_region_map(rx, rz)?.unwrap_or_default();
        map.insert(local_index(col), payload);
        let blob = serialize_region(&map);
        let tmp = self.dir.join(format!("r.{rx}.{rz}.gcr.tmp"));
        std::fs::write(&tmp, &blob)?;
        std::fs::rename(&tmp, self.region_path(rx, rz))?;
        Ok(())
    }

    /// Every column with a saved payload, by scanning the region files. Used at
    /// startup so streaming knows which columns to load instead of generate.
    pub fn scan_saved(&self) -> HashSet<ColumnPos> {
        let mut out = HashSet::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return out; // dir absent yet → nothing saved
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(region) = parse_region_filename(name) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            let Some(map) = parse_region(&bytes) else {
                continue;
            };
            for &index in map.keys() {
                out.insert(column_at(region, index));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{BlockId, GRASS, STONE};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gitcraft_region_{tag}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Eight sections with a couple of distinguishing edits.
    fn sample_column() -> Vec<Arc<Section>> {
        (0..COLUMN_SECTIONS)
            .map(|i| {
                let mut s = Section::empty();
                s.set(i, i, i, BlockId(i as u16 + 1));
                s.set(31, 0, 31, STONE);
                Arc::new(s)
            })
            .collect()
    }

    #[test]
    fn column_payload_roundtrips() {
        let cols = sample_column();
        let bytes = serialize_column(&cols);
        let back = deserialize_column(&bytes).expect("valid column payload");
        assert_eq!(back.len(), COLUMN_SECTIONS);
        for (a, b) in cols.iter().zip(&back) {
            assert_eq!(**a, *b);
        }
        // Trailing garbage is rejected.
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(deserialize_column(&extra).is_none());
    }

    #[test]
    fn coordinates_roundtrip_including_negatives() {
        for c in [
            ColumnPos { x: 0, z: 0 },
            ColumnPos { x: 5, z: 31 },
            ColumnPos { x: 32, z: 64 },
            ColumnPos { x: -1, z: -1 },
            ColumnPos { x: -33, z: 40 },
            ColumnPos { x: -100, z: -250 },
        ] {
            let idx = local_index(c);
            assert!(idx < 1024, "local index {idx} out of range for {c:?}");
            assert_eq!(column_at(region_of(c), idx), c);
        }
    }

    #[test]
    fn region_blob_roundtrips_and_rejects_garbage() {
        let mut map = BTreeMap::new();
        assert_eq!(parse_region(&serialize_region(&map)).unwrap().len(), 0);
        map.insert(3u16, vec![1, 2, 3]);
        map.insert(1000u16, vec![9; 40]);
        let blob = serialize_region(&map);
        assert_eq!(parse_region(&blob).unwrap(), map);
        assert!(parse_region(b"XXXX").is_none()); // bad magic
        assert!(parse_region(&blob[..blob.len() - 1]).is_none()); // truncated
    }

    #[test]
    fn parse_region_rejects_wrong_version() {
        // Build a valid blob then patch the version field (bytes 4..6) to 2.
        let map: BTreeMap<u16, Vec<u8>> = BTreeMap::new();
        let mut blob = serialize_region(&map);
        // VERSION is at bytes 4-5 (after the 4-byte magic).
        blob[4] = 2;
        blob[5] = 0;
        assert!(
            parse_region(&blob).is_none(),
            "parse_region must reject a blob with version != VERSION"
        );
    }

    #[test]
    fn store_saves_loads_and_scans() {
        let store = RegionStore::new(temp_dir("save_load_scan"));
        let a = ColumnPos { x: 2, z: 3 };
        let b = ColumnPos { x: 5, z: 3 }; // same region as a
        let far = ColumnPos { x: 100, z: -40 }; // different region

        assert!(store.load_column(a).unwrap().is_none()); // nothing yet
        assert!(store.scan_saved().is_empty());

        let payload_a = serialize_column(&sample_column());
        store.save_column(a, payload_a.clone()).unwrap();
        store
            .save_column(b, serialize_column(&sample_column()))
            .unwrap();
        store
            .save_column(far, serialize_column(&sample_column()))
            .unwrap();

        assert_eq!(store.load_column(a).unwrap(), Some(payload_a));
        assert!(store.load_column(b).unwrap().is_some());
        assert!(store.load_column(far).unwrap().is_some());

        let saved = store.scan_saved();
        assert_eq!(saved.len(), 3);
        assert!(saved.contains(&a) && saved.contains(&b) && saved.contains(&far));
    }

    #[test]
    fn saving_a_column_twice_replaces_it() {
        let store = RegionStore::new(temp_dir("overwrite"));
        let c = ColumnPos { x: 1, z: 1 };
        store.save_column(c, vec![1, 2, 3]).unwrap();

        let mut updated = Section::empty();
        updated.set(0, 0, 0, GRASS);
        let updated = Arc::new(updated);
        let new_payload = serialize_column(&vec![updated; COLUMN_SECTIONS]);
        store.save_column(c, new_payload.clone()).unwrap();

        assert_eq!(store.load_column(c).unwrap(), Some(new_payload));
        assert_eq!(store.scan_saved().len(), 1); // still one column, not two
    }
}
