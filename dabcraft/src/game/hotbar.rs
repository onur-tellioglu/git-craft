use crate::world::block::{BlockId, PLACEABLE};

pub const SLOT_COUNT: usize = 9;

/// Creative hotbar: 9 slots over the PLACEABLE list. Keys 1–9 / wheel pick
/// a slot; shift+wheel pages the whole bar through every placeable block
/// (spec §7). Pages wrap; a short last page wraps around to the list front
/// so no slot is ever empty.
pub struct Hotbar {
    pub slots: [BlockId; SLOT_COUNT],
    pub selected: usize,
    page: usize,
}

impl Hotbar {
    pub fn new() -> Self {
        let mut hb = Self { slots: [PLACEABLE[0]; SLOT_COUNT], selected: 0, page: 0 };
        hb.fill_from_page();
        hb
    }

    fn page_count() -> usize {
        PLACEABLE.len().div_ceil(SLOT_COUNT)
    }

    fn fill_from_page(&mut self) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            *slot = PLACEABLE[(self.page * SLOT_COUNT + i) % PLACEABLE.len()];
        }
    }

    /// Keys 1–9 (0-based). Out-of-range is ignored.
    pub fn select(&mut self, slot: usize) {
        if slot < SLOT_COUNT {
            self.selected = slot;
        }
    }

    /// Mouse wheel: cycle the selected slot (positive = next, wraps).
    pub fn scroll(&mut self, steps: i32) {
        self.selected =
            (self.selected as i32 + steps).rem_euclid(SLOT_COUNT as i32) as usize;
    }

    /// Shift+wheel: page the bar through the full placeable list (wraps).
    pub fn page_scroll(&mut self, steps: i32) {
        let pages = Self::page_count() as i32;
        self.page = (self.page as i32 + steps).rem_euclid(pages) as usize;
        self.fill_from_page();
    }

    pub fn selected_block(&self) -> BlockId {
        self.slots[self.selected]
    }
}

impl Default for Hotbar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{CACTUS, GRASS, SPRUCE_LEAVES, SPRUCE_LOG, TORCH, WATER};

    #[test]
    fn defaults_to_first_nine_placeable_blocks() {
        let hb = Hotbar::new();
        assert_eq!(hb.slots[0], GRASS);
        assert_eq!(hb.slots[5], WATER);
        assert_eq!(hb.slots[8], SPRUCE_LOG);
        assert_eq!(hb.selected, 0);
        assert_eq!(hb.selected_block(), GRASS);
    }

    #[test]
    fn select_clamps_to_valid_slots() {
        let mut hb = Hotbar::new();
        hb.select(8);
        assert_eq!(hb.selected, 8);
        hb.select(9); // out of range: ignored
        assert_eq!(hb.selected, 8);
    }

    #[test]
    fn scroll_cycles_selection_both_directions() {
        let mut hb = Hotbar::new();
        hb.scroll(1);
        assert_eq!(hb.selected, 1);
        hb.scroll(-2);
        assert_eq!(hb.selected, 8, "wraps backwards");
        hb.scroll(1);
        assert_eq!(hb.selected, 0, "wraps forwards");
    }

    #[test]
    fn page_scroll_swaps_in_the_remaining_blocks_with_wraparound_fill() {
        let mut hb = Hotbar::new();
        hb.page_scroll(1);
        // Page 1 = PLACEABLE[9..11] (SPRUCE_LEAVES, CACTUS, TORCH) then wraps to the front.
        assert_eq!(hb.slots[0], SPRUCE_LEAVES);
        assert_eq!(hb.slots[1], CACTUS);
        assert_eq!(hb.slots[2], TORCH);
        assert_eq!(hb.slots[3], GRASS, "wraparound fill keeps slots populated");
        hb.page_scroll(1);
        assert_eq!(hb.slots[0], GRASS, "two pages total, wraps back to page 0");
        hb.page_scroll(-1);
        assert_eq!(hb.slots[0], SPRUCE_LEAVES, "negative paging wraps too");
    }

    #[test]
    fn selection_survives_paging() {
        let mut hb = Hotbar::new();
        hb.select(4);
        hb.page_scroll(1);
        assert_eq!(hb.selected, 4);
    }
}
