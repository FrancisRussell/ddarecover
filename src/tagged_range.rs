use std::collections::{btree_map, BTreeMap};
use std::ops::Range;
use std::cmp;

#[derive(Clone, Debug)]
pub struct TaggedRange<T> {
    starts: BTreeMap<u64, InternalRegion<T>>,
}

#[derive(Clone, Debug)]
struct InternalRegion<T> {
    length: u64,
    tag: T,
}

#[derive(Clone, Debug)]
pub struct Region<T> {
    pub start: u64,
    pub length: u64,
    pub tag: T,
}

impl<T> Region<T> {
    pub fn new(start: u64, length: u64, tag: T) -> Region<T> {
        Region {
            start: start,
            length: length,
            tag: tag,
        }
    }

    pub fn as_range(&self) -> Range<u64> {
        self.start..(self.start + self.length)
    }
}

impl<T> TaggedRange<T> {
    pub fn new() -> TaggedRange<T> {
        TaggedRange {
            starts: BTreeMap::new(),
        }
    }

    pub fn put(&mut self, range: Range<u64>, tag: T) where T: Clone {
        assert!(range.end >= range.start);
        if range.end == range.start {
            return;
        }
        let covering_range = self.get_covering_range(&range);
        let overlaps: Vec<u64> = self.starts.range(covering_range).map(|(idx, _)| *idx).collect();
        for start in overlaps.iter().cloned() {
            let region = self.starts.remove(&start).unwrap();
            if start < range.start {
                assert!(start + region.length > range.start);
                let new_region = InternalRegion {
                    length: range.start - start,
                    tag: region.tag.clone(),
                };
                self.starts.insert(start, new_region);
            }
            if start + region.length > range.end {
                assert!(start < range.end);
                let new_region = InternalRegion {
                    length: start + region.length - range.end,
                    tag: region.tag,
                };
                self.starts.insert(range.end, new_region);
            }
        }
        let new_region = InternalRegion {
            length: range.end - range.start,
            tag: tag,
        };
        self.starts.insert(range.start, new_region);
    }

    fn get_covering_range(&self, range: &Range<u64>) -> Range<u64> {
        let start = match self.starts.range(0..range.start).rev().next() {
            Some((start, region)) => if *start + region.length > range.start {
                *start
            } else {
                range.start
            },
            None => range.start,
        };
        start..range.end
    }

    pub fn iter<'a>(&'a self) -> Iter<'a, T> where T: Clone {
        self.into_iter()
    }

    pub fn iter_range<'a>(&'a self, range: Range<u64>) -> Iter<'a, T> where T: Clone {
        let covering_range = self.get_covering_range(&range);
        let iter = self.starts.range(covering_range);
        Iter {
            range: range.clone(),
            iter: iter,
        }
    }
}

impl<'a, T> IntoIterator for &'a TaggedRange<T> where T: Clone {
    type Item = Region<T>;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        let iter = self.starts.range(..);
        let range = 0..u64::max_value();
        Iter {
            range: range,
            iter: iter,
        }
    }
}

pub struct Iter<'a, T> where T: 'a {
    range: Range<u64>,
    iter: btree_map::Range<'a, u64, InternalRegion<T>>,
}

impl<'a, T> Iterator for Iter<'a, T> where T: Clone {
    type Item = Region<T>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.iter.next() {
            Some((start, iregion)) => {
                let start_restricted = cmp::max(*start, self.range.start);
                let end_restricted = cmp::min(start + iregion.length, self.range.end);
                let region = Region {
                    start: start_restricted,
                    length: end_restricted - start_restricted,
                    tag: iregion.tag.clone(),
                };
                Some(region)
            },
            None => None
        }
    }
}
