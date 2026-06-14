/// First-fit slab allocator over an abstract range of slots; the terrain
/// renderer maps slots to quads in one big storage buffer. Free list is
/// offset-sorted (offset, len) with full coalescing on free.
pub struct Arena {
    free: Vec<(u32, u32)>,
    capacity: u32,
    used: u32,
}

impl Arena {
    pub fn new(capacity: u32) -> Self {
        Self {
            free: vec![(0, capacity)],
            capacity,
            used: 0,
        }
    }

    pub fn alloc(&mut self, len: u32) -> Option<u32> {
        if len == 0 {
            return None;
        }
        let i = self.free.iter().position(|&(_, l)| l >= len)?;
        let (off, l) = self.free[i];
        if l == len {
            self.free.remove(i);
        } else {
            self.free[i] = (off + len, l - len);
        }
        self.used += len;
        Some(off)
    }

    /// Free a range previously returned by `alloc` (same len).
    pub fn free(&mut self, offset: u32, len: u32) {
        debug_assert!(offset + len <= self.capacity);
        let i = self.free.partition_point(|&(o, _)| o < offset);
        self.free.insert(i, (offset, len));
        self.used -= len;
        // Coalesce with the next range, then with the previous one.
        if i + 1 < self.free.len() && self.free[i].0 + self.free[i].1 == self.free[i + 1].0 {
            self.free[i].1 += self.free[i + 1].1;
            self.free.remove(i + 1);
        }
        if i > 0 && self.free[i - 1].0 + self.free[i - 1].1 == self.free[i].0 {
            self.free[i - 1].1 += self.free[i].1;
            self.free.remove(i);
        }
    }

    pub fn used(&self) -> u32 {
        self.used
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_from_the_start() {
        let mut a = Arena::new(100);
        assert_eq!(a.alloc(10), Some(0));
        assert_eq!(a.alloc(20), Some(10));
        assert_eq!(a.used(), 30);
    }

    #[test]
    fn refuses_when_no_hole_fits() {
        let mut a = Arena::new(100);
        assert_eq!(a.alloc(60), Some(0));
        assert_eq!(a.alloc(50), None, "only 40 left");
        assert_eq!(a.used(), 60, "failed alloc must not change accounting");
    }

    #[test]
    fn freed_ranges_are_reused() {
        let mut a = Arena::new(100);
        let x = a.alloc(40).unwrap();
        a.alloc(40).unwrap();
        a.free(x, 40);
        assert_eq!(a.alloc(30), Some(0), "first-fit lands in the freed hole");
    }

    #[test]
    fn adjacent_frees_coalesce() {
        let mut a = Arena::new(100);
        let x = a.alloc(30).unwrap();
        let y = a.alloc(30).unwrap();
        let z = a.alloc(40).unwrap();
        assert_eq!((x, y, z), (0, 30, 60));
        a.free(x, 30);
        a.free(z, 40);
        a.free(y, 30); // middle free must merge with BOTH neighbors
        assert_eq!(a.used(), 0);
        assert_eq!(a.alloc(100), Some(0), "fully coalesced back to one range");
    }

    #[test]
    fn zero_len_alloc_is_rejected() {
        let mut a = Arena::new(100);
        assert_eq!(a.alloc(0), None);
    }

    #[test]
    fn exact_fit_consumes_the_hole() {
        let mut a = Arena::new(50);
        assert_eq!(a.alloc(50), Some(0));
        assert_eq!(a.alloc(1), None);
        a.free(0, 50);
        assert_eq!(a.alloc(50), Some(0));
    }
}
