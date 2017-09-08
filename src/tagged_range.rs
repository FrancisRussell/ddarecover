use std::collections::BTreeMap;
use std::ops::Range;

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
        let mut overlaps: Vec<u64> = self.starts.range(range.clone()).map(|(idx, _)| *idx).collect();
        match self.starts.range(0..range.start).rev().next() {
            Some((start, region)) => if start + region.length > range.start {
                overlaps.push(*start);
            },
            None => {},
        };
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
}

impl<'a, T> IntoIterator for &'a TaggedRange<T> where T: Clone {
    type Item = Region<T>;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        Iter {
            iter: self.starts.iter(),
        }
    }
}

pub struct Iter<'a, T> where T: 'a {
    iter: <&'a BTreeMap<u64, InternalRegion<T>> as IntoIterator>::IntoIter,
}

impl<'a, T> Iterator for Iter<'a, T> where T: Clone {
    type Item = Region<T>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.iter.next() {
            Some((start, iregion)) => Some(Region {
                start: *start,
                length: iregion.length,
                tag: iregion.tag.clone(),
            }),
            None => None
        }
    }
}
