use std::cell::RefCell;

pub struct Lease<'a> {
    range: Range,
    manager: &'a Manager,
}
impl<'a> Lease<'a> {
    fn new(range: Range, manager: &'a Manager) -> Self {
        Self { range, manager }
    }
}
impl<'a> Drop for Lease<'a> {
    fn drop(&mut self) {
        self.manager.return_range(&self.range);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    start: usize,
    len: usize,
}
impl Range {
    /// Create a new range from min to max exclusive
    pub fn new_from_min_max_exclusive(min: usize, max: usize) -> Self {
        Self {
            start: min,
            len: max - min + 1,
        }
    }
    // read:  |------|
    // write:    |--------|
    // We need to turn this into:
    // read:  |--|
    // write:    |--------|
    // So that the ranges do not overlap.
    pub fn make_non_overlapping(read_range: Range, write_range: Range) -> (Self, Self) {
        assert!(read_range.start <= write_range.start);
        if read_range.start + read_range.len <= write_range.start {
            // No overlap
            (read_range, write_range)
        } else {
            // Overlap
            let new_read_range = Range {
                start: read_range.start,
                len: write_range.start - read_range.start
            };
            // write range doesn't change
            (new_read_range, write_range)
        }
    }

    pub fn empty() -> Self {
        Self { start: 0, len: 0 }
    }
    pub fn new(start: usize, len: usize) -> Self {
        Self { start, len }
    }
    pub fn grow(&self, grow_by: usize) -> Self {
        Self {
            start: self.start,
            len: self.len + grow_by,
        }
    }
    pub fn start(&self) -> usize {
        self.start
    }
    pub fn shrink(&self, shrink_by: usize) -> Self {
        Self {
            start: self.start,
            len: self.len - shrink_by,
        }
    }
    pub fn merge_with(&self, other: &Self) -> Option<Self> {
        if self.start + self.len == other.start {
            // Tail merge - self:other
            Some(Self {
                start: self.start,
                len: self.len + other.len,
            })
        } else if other.start + other.len == self.start {
            // Head merge - other:self
            Some(Self {
                start: other.start,
                len: self.len + other.len,
            })
        } else {
            None
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    // Returns true of other is fully contained inside self
    fn fully_contains(&self, other: &Self) -> bool {
        self.start <= other.start && self.start + self.len >= other.start + other.len
    }

    // fn intersects_with(&self, other: &Self) -> bool {
    //     if other.start >= self.start && other.start < self.start + self.len {
    //         // other starts inside self
    //         true
    //     } else if other.start + other.len >= self.start && other.start + other.len < self.start + self.len {
    //         // other ends inside self
    //         true
    //     } else if self.start >= other.start && self.start < other.start + other.len {
    //         // self starts inside other
    //         true
    //     } else if self.start + self.len >= other.start && self.start + self.len < other.start + other.len {
    //         // self ends inside other
    //         true
    //     } else {
    //         false
    //     }
    // }

    // given our range, if the range starting at start of length len is fully
    // contained inside our range, split this into the range before the given
    // range and the range after the given range.
    fn split_into(&self, other: &Range) -> Option<(Self, Self)> {
        if self.fully_contains(other) {
            // Create three ranges, before, contained, and after
            let before = Self {
                start: self.start,
                len: other.start - self.start,
            };
            let sp = other.start + other.len;
            let after = Self {
                start: sp,
                len: self.start + self.len - sp,
            };
            Some((before, after))
        } else {
            None
        }
    }

    pub fn contains(&self, input_index: usize) -> bool {
        if input_index >= self.start && input_index < self.start + self.len {
            true
        } else {
            false
        }
    }
}

/// Manager controls a contiguous range of space.  The space begins with a
/// contiguous range of writeable space.  Sub-ranges of the writable space
/// can be "checked out" for exclusive reading/writing by that leasee.  When
/// the lease is dropped, the space is returned to the manager.
///
/// The returned lease range is then considered read-only and not writable
/// again.  
#[derive(Debug)]
pub struct Manager {
    read_ranges: RefCell<Vec<Range>>,
    write_ranges: RefCell<Vec<Range>>,
}
impl Manager {
    /// Constructor
    pub fn new(len: usize) -> Self {
        let full_range = Range::new(0, len);

        Self {
            read_ranges: RefCell::new(vec![]),
            write_ranges: RefCell::new(vec![full_range]),
        }
    }

    /// How much total space is available for writing?
    pub fn write_available(&self) -> usize {
        self.write_ranges
            .borrow()
            .iter()
            .map(|r| r.len)
            .sum::<usize>()
    }

    pub fn read_available(&self) -> usize {
        self.read_ranges
            .borrow()
            .iter()
            .map(|r| r.len)
            .sum::<usize>()
    }

    /// Request to get a read slice.  The slice is not "checked out", but
    /// a simple Ok(()) is returned to indicate that you can write to this.
    /// and shall not be returned.
    pub fn get_read_slice<'a>(&'a mut self, slice: &Range) -> Option<()> {
        // An odd edge case where we don't actually need any data
        if slice.is_empty() {
            return Some(());
        }
        // Check that this is contined inside an existing read range
        let read_ranges = self.read_ranges.borrow_mut();
        read_ranges
            .iter()
            .find(|r| r.fully_contains(&slice))
            .map(|_| ())
    }

    /// Check out a writable range.  The range must be fully contained in a
    /// writable range.  If it is, the range is removed from the writable
    /// ranges and a lease is returned.  When the lease is dropped, the
    /// range is returned to the manager.
    pub fn check_out_write_slice<'a>(&'a self, slice: Range) -> Option<Lease<'a>> {
        let range = Self::extract_slice(&mut self.write_ranges.borrow_mut(), slice)?;
        Some(Lease::new(range, self))
    }
    /// A self-managed checkout.  It's up to the caller to properly return this lease
    pub fn check_out_write_slice_self_managed<'a>(&'a self, slice: Range) -> Option<Range> {
        let range = Self::extract_slice(&mut self.write_ranges.borrow_mut(), slice)?;
        Some(range)
    }

    /// Given a set of ranges, find a range that fully contains the given range.
    /// Remove the hole from the ranges, and return the range that was contained.
    fn extract_slice(ranges: &mut Vec<Range>, requested: Range) -> Option<Range> {
        let (index, (before, after)) = ranges
            .iter()
            .enumerate()
            .find_map(|(i, range)| range.split_into(&requested).map(|v| (i, v)))?;

        // Found one.  Remove that and replace it with the segments returned by
        // split into (if they're not empty)
        ranges.remove(index);
        if !before.is_empty() {
            ranges.push(before);
        }
        if !after.is_empty() {
            ranges.push(after);
        }
        Some(requested)
    }

    fn merge_range_with_ranges(ranges: &mut Vec<Range>, to_merge: Range) {
        // This operation must perform merges with the set of ranges over
        // and over again until it can't merge any more.
        // This is to ensure ranges in our list are always fully contiguous.
        let mut to_merge = to_merge; // make mutable because we keep growing this
        loop {
            // Try to merge with one of the ranges
            let merged = ranges
                .iter()
                .enumerate()
                .find_map(|(i, range)| Some((i, range.merge_with(&to_merge)?)));

            // If we merged, remove the old range
            match merged {
                Some((i, merged)) => {
                    ranges.remove(i);
                    to_merge = merged;
                }
                None => {
                    // No more to merge, add the (possibly merged) range to the list
                    ranges.push(to_merge);
                    break;
                }
            }
        }
    }

    /// Called by the leasee when the lease is dropped.  The range is returned
    /// to the manager and added to the read-only side of the available ranges.
    pub fn return_range(&self, slice: &Range) {
        // Slices once returned become read only.
        Self::merge_range_with_ranges(&mut self.read_ranges.borrow_mut(), slice.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let manager = Manager::new(1024);
        assert_eq!(manager.write_available(), 1024);
    }

    #[test]
    fn test_split() {
        let range = Range::new(0, 10);
        let (before, after) = range.split_into(&Range::new(2, 3)).unwrap();
        assert_eq!(before, Range::new(0, 2));
        assert_eq!(after, Range::new(5, 5));

        // missed
        let range = Range::new(0, 10);
        let res = range.split_into(&Range::new(2, 9));
        assert!(res.is_none());

        // near miss
        let range = Range::new(10, 10);
        let res = range.split_into(&Range::new(5, 10));
        assert!(res.is_none());

        // bigger
        let range = Range::new(10, 10);
        let res = range.split_into(&Range::new(5, 20));
        assert!(res.is_none());
    }

    #[test]
    fn test_manager() {
        let mut manager = Manager::new(1000);
        assert_eq!(manager.write_available(), 1000);
        assert_eq!(manager.read_available(), 0);

        // Check out a slice
        let slice = Range::new(0, 10);

        // Try to check out near read range
        assert!(manager.get_read_slice(&slice).is_none());
        {
            let _lease = manager.check_out_write_slice(slice).unwrap();
            assert_eq!(manager.write_available(), 990);
            assert_eq!(manager.read_available(), 0);
        }
        assert_eq!(manager.write_available(), 990);
        assert_eq!(manager.read_available(), 10);
        // Read is available now
        assert!(manager.get_read_slice(&slice).is_some());
        // Can get less
        assert!(manager.get_read_slice(&slice.shrink(5)).is_some());
        // Cannot get more
        assert!(manager.get_read_slice(&slice.grow(1)).is_none());
    }

    #[test]
    fn test_overlapping() {
        let read_range = Range::new_from_min_max_exclusive(0,5);
        let write_range = Range::new_from_min_max_exclusive(6,10);
        let (new_read,new_write) = Range::make_non_overlapping(read_range, write_range);
        assert_eq!(read_range, new_read);
        assert_eq!(write_range, new_write);

        let read_range = Range::new_from_min_max_exclusive(0,6);
        let write_range = Range::new_from_min_max_exclusive(6,10);
        let (new_read,new_write) = Range::make_non_overlapping(read_range, write_range);
        assert_ne!(read_range, new_read);
        assert_eq!(new_read, Range::new_from_min_max_exclusive(0,5));
        assert_eq!(write_range, new_write);
    }
}
