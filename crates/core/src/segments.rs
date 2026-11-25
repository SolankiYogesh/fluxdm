//! Dynamic file segmentation.
//!
//! A download starts as one segment covering the whole file. Whenever a
//! connection is free it first adopts an unowned unfinished segment; if there is
//! none, it halves the segment with the most bytes still outstanding and takes
//! the upper half. Fast connections therefore keep helping slow ones until the
//! very end of the transfer.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    /// First byte of the segment.
    pub start: u64,
    /// One past the last byte. `u64::MAX` when the size is unknown.
    pub end: u64,
    /// Bytes safely written to disk.
    pub done: u64,
    /// Bytes handed to a writer (>= done). Splits must never cut below this.
    #[serde(skip)]
    claimed: u64,
    #[serde(skip)]
    pub active: bool,
}

impl Segment {
    fn new(start: u64, end: u64) -> Self {
        Self { start, end, done: 0, claimed: 0, active: false }
    }
    fn next_claim(&self) -> u64 {
        self.start + self.claimed.max(self.done)
    }
    pub fn unclaimed(&self) -> u64 {
        self.end.saturating_sub(self.next_claim())
    }
    pub fn position(&self) -> u64 {
        self.start + self.done
    }
    pub fn is_complete(&self) -> bool {
        self.position() >= self.end
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SegmentMap {
    pub segs: Vec<Segment>,
}

impl SegmentMap {
    pub fn new(total: Option<u64>) -> Self {
        Self { segs: vec![Segment::new(0, total.unwrap_or(u64::MAX))] }
    }

    /// Forget ownership after a stop or restart; only durable progress remains.
    pub fn reset_runtime(&mut self) {
        for s in &mut self.segs {
            s.active = false;
            s.claimed = s.done;
        }
    }

    pub fn downloaded(&self) -> u64 {
        self.segs.iter().map(|s| s.done).sum()
    }

    pub fn is_complete(&self) -> bool {
        !self.segs.is_empty() && self.segs.iter().all(Segment::is_complete)
    }

    pub fn active_count(&self) -> usize {
        self.segs.iter().filter(|s| s.active).count()
    }

    /// Mark a specific segment as owned.
    pub fn take(&mut self, idx: usize) {
        self.segs[idx].active = true;
    }

    /// Find work for a free connection. `min_split` is the smallest piece worth
    /// opening a connection for.
    pub fn next_work(&mut self, min_split: u64) -> Option<usize> {
        if let Some(i) = self.segs.iter().position(|s| !s.active && !s.is_complete()) {
            self.take(i);
            return Some(i);
        }
        self.split_largest(min_split.max(1))
    }

    fn split_largest(&mut self, min_split: u64) -> Option<usize> {
        let (idx, remaining) = self
            .segs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.active && s.end != u64::MAX)
            .map(|(i, s)| (i, s.unclaimed()))
            .max_by_key(|&(_, r)| r)?;
        if remaining < min_split.saturating_mul(2) {
            return None;
        }
        let end = self.segs[idx].end;
        let mid = end - remaining / 2;
        self.segs[idx].end = mid;
        let mut upper = Segment::new(mid, end);
        upper.active = true;
        self.segs.push(upper);
        Some(self.segs.len() - 1)
    }

    /// Claim up to `len` bytes for writing. Returns the file offset and how many
    /// bytes may be written; `None` once the segment (possibly shrunk by a
    /// split) has no room left.
    pub fn reserve(&mut self, idx: usize, len: usize) -> Option<(u64, usize)> {
        let s = &mut self.segs[idx];
        s.claimed = s.claimed.max(s.done);
        let offset = s.start + s.claimed;
        let room = s.end.saturating_sub(offset);
        let n = (len as u64).min(room) as usize;
        if n == 0 {
            return None;
        }
        s.claimed += n as u64;
        Some((offset, n))
    }

    pub fn commit(&mut self, idx: usize, n: usize) {
        self.segs[idx].done += n as u64;
    }

    pub fn release(&mut self, idx: usize) {
        let s = &mut self.segs[idx];
        s.active = false;
        s.claimed = s.done;
    }

    /// Close an unknown-size segment at end of stream.
    pub fn seal(&mut self, idx: usize) {
        let s = &mut self.segs[idx];
        s.end = s.start + s.done;
    }

    /// True when the segments tile `0..total` with no gap or overlap.
    pub fn covers(&self, total: u64) -> bool {
        let mut ranges: Vec<(u64, u64)> = self.segs.iter().map(|s| (s.start, s.end)).collect();
        ranges.sort_unstable();
        let mut at = 0;
        for (start, end) in ranges {
            if start != at || end < start {
                return false;
            }
            at = end;
        }
        at == total
    }

    pub fn progress_is_sane(&self) -> bool {
        self.segs.iter().all(|s| s.start <= s.end && s.done <= s.end - s.start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_the_largest_remaining_segment() {
        let mut m = SegmentMap::new(Some(1000));
        assert_eq!(m.next_work(10), Some(0));
        let b = m.next_work(10).unwrap();
        assert_eq!((m.segs[b].start, m.segs[b].end), (500, 1000));
        // Segment 0 makes progress, so segment b now has the most left.
        let (_, n) = m.reserve(0, 400).unwrap();
        m.commit(0, n);
        let c = m.next_work(10).unwrap();
        assert_eq!((m.segs[c].start, m.segs[c].end), (750, 1000));
        assert!(m.covers(1000));
    }

    #[test]
    fn split_never_cuts_into_claimed_bytes() {
        let mut m = SegmentMap::new(Some(1000));
        m.next_work(10);
        let (off, n) = m.reserve(0, 800).unwrap();
        assert_eq!((off, n), (0, 800));
        let b = m.next_work(10).unwrap();
        assert!(m.segs[b].start >= 800);
        assert!(m.covers(1000));
    }

    #[test]
    fn stops_splitting_below_minimum() {
        let mut m = SegmentMap::new(Some(100));
        m.next_work(60);
        assert_eq!(m.next_work(60), None);
    }

    #[test]
    fn shrunk_segment_truncates_writes() {
        let mut m = SegmentMap::new(Some(1000));
        m.next_work(10);
        m.next_work(10);
        assert_eq!(m.reserve(0, 900), Some((0, 500)));
        assert_eq!(m.reserve(0, 1), None);
    }

    #[test]
    fn released_segment_is_adopted_from_durable_position() {
        let mut m = SegmentMap::new(Some(1000));
        m.next_work(10);
        let (_, n) = m.reserve(0, 100).unwrap();
        m.commit(0, n);
        m.reserve(0, 100); // claimed but never written
        m.release(0);
        assert_eq!(m.next_work(10), Some(0));
        assert_eq!(m.reserve(0, 5), Some((100, 5)));
    }

    #[test]
    fn unknown_size_is_never_split() {
        let mut m = SegmentMap::new(None);
        m.next_work(1);
        assert_eq!(m.next_work(1), None);
    }

    #[test]
    fn randomised_schedule_keeps_the_file_tiled() {
        let total = 1_000_003u64;
        let mut m = SegmentMap::new(Some(total));
        let mut seen = vec![0u8; total as usize];
        let mut owned: Vec<usize> = vec![m.next_work(512).unwrap()];
        let mut x = 0x2545F4914F6CDD1Du64;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        while !m.is_complete() {
            if owned.len() < 8 && rnd() % 3 == 0 {
                if let Some(i) = m.next_work(512) {
                    owned.push(i);
                }
            }
            if owned.is_empty() {
                owned.extend(m.next_work(512));
            }
            let k = rnd() as usize % owned.len();
            let i = owned[k];
            if rnd() % 50 == 0 {
                m.release(i);
                owned.swap_remove(k);
                continue;
            }
            match m.reserve(i, 1 + rnd() as usize % 5000) {
                Some((off, n)) => {
                    seen[off as usize..off as usize + n].iter_mut().for_each(|b| *b += 1);
                    m.commit(i, n);
                }
                None => {
                    m.release(i);
                    owned.swap_remove(k);
                }
            }
            assert!(m.covers(total) && m.progress_is_sane());
        }
        assert!(seen.iter().all(|&b| b == 1), "every byte written exactly once");
    }
}
