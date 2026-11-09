//! Source-content similarity for file move detection.
//!
//! The score is `floor(100 * common / max(old_bytes, new_bytes))`, where
//! `common` is the UTF-8 byte length of equal units in a deterministic Myers
//! diff. Both sources use Unicode scalar values as units when both are at or
//! below 4 KiB; if either side is larger, both use complete lines, including
//! line endings. The threshold is applied by the caller; this module only
//! computes the source score.
//!
//! [`source_similarity_at_least`] can reject a pair early when a frequency
//! upper bound proves that no alignment can reach the requested threshold.
//! That is a safe prune, not a different score: it only skips pairs whose exact
//! score is below the threshold.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use similar::{Algorithm, DiffOp, TextDiff, capture_diff_slices};

/// Sources at or below this many bytes on both sides use Unicode scalars.
pub const SCALAR_UNIT_MAX_BYTES: usize = 4 * 1024;

// LCS is only an optimization. If its scratch would exceed this bound, or a
// Hunt-Szymanski match set is too large, the second bound is skipped and the
// unchanged Myers score is computed instead. No candidate is made ineligible.
const LCS_SCRATCH_BUDGET: usize = 32 * 1024 * 1024;
const LCS_HUNT_MAX_PAIRS: usize = 100_000;

/// The unit kind a prepared source was tokenized with.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum UnitMode {
    Scalar,
    Line,
}

/// A comparison-wide interner for prepared source units.
///
/// Unit identity is content, so the same text has the same id in every file of
/// one comparison. Prepared sources never own the original bytes.
/// A bound for prepared-profile and interner retention. When it is exceeded,
/// callers fall back to the unchanged byte-slice scorer for the unprepared
/// pair; this is an optimization budget, never a candidate or score cap.
#[derive(Clone, Copy, Debug)]
pub struct ProfileBudget {
    pub max_units: usize,
    pub max_light_entries: usize,
    pub max_heavy_entries: usize,
    pub max_interned_bytes: usize,
}

impl Default for ProfileBudget {
    fn default() -> Self {
        Self {
            max_units: 8_000_000,
            max_light_entries: 5_000_000,
            max_heavy_entries: 2_000_000,
            max_interned_bytes: 8 * 1024 * 1024,
        }
    }
}

/// A comparison-wide interner for prepared source units.
///
/// Unit identity is content, so the same text has the same id in every file of
/// one comparison. Prepared sources never own the original bytes.
#[derive(Default)]
pub struct SimilarityInterner {
    ids: HashMap<Box<str>, u32>,
    unit_bytes: Vec<usize>,
    prepared_units: usize,
    prepared_light_entries: usize,
    prepared_heavy_entries: usize,
    interned_bytes: usize,
}

impl SimilarityInterner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tokenize `source` once and intern every unit, without a budget.
    pub fn prepare(&mut self, source: &str, mode: UnitMode) -> PreparedSource {
        self.prepare_inner(source, mode, None)
            .expect("an unbudgeted preparation always succeeds")
    }

    /// Tokenize `source` only while the interner/profile budget permits it.
    pub fn try_prepare(
        &mut self,
        source: &str,
        mode: UnitMode,
        budget: ProfileBudget,
    ) -> Option<PreparedSource> {
        self.prepare_inner(source, mode, Some(budget))
    }

    fn prepare_inner(
        &mut self,
        source: &str,
        mode: UnitMode,
        budget: Option<ProfileBudget>,
    ) -> Option<PreparedSource> {
        let unit_count = match mode {
            UnitMode::Scalar => source.chars().count(),
            UnitMode::Line => source.split_inclusive('\n').count(),
        };
        if let Some(budget) = budget
            && self.prepared_units.saturating_add(unit_count) > budget.max_units
        {
            return None;
        }

        let mut ids = Vec::with_capacity(unit_count);
        let mut counts: HashMap<u32, usize> = HashMap::new();
        let mut lengths: HashMap<usize, usize> = HashMap::new();
        let mut total_len = 0;

        let mut intern = |unit: &str| {
            let id = if let Some(&id) = self.ids.get(unit) {
                id
            } else {
                let cost = unit.len().saturating_add(32);
                if let Some(budget) = budget
                    && self.interned_bytes.saturating_add(cost) > budget.max_interned_bytes
                {
                    return None;
                }
                let id = self.unit_bytes.len() as u32;
                self.ids.insert(unit.into(), id);
                self.unit_bytes.push(unit.len());
                self.interned_bytes = self.interned_bytes.saturating_add(cost);
                id
            };
            *counts.entry(id).or_default() += 1;
            *lengths.entry(unit.len()).or_default() += 1;
            total_len += unit.len();
            Some(id)
        };

        match mode {
            UnitMode::Scalar => {
                for (index, scalar) in source.char_indices() {
                    ids.push(intern(&source[index..index + scalar.len_utf8()])?);
                }
            }
            UnitMode::Line => {
                for unit in source.split_inclusive('\n') {
                    ids.push(intern(unit)?);
                }
            }
        }

        let mut heavy: Vec<(u32, usize)> = counts
            .iter()
            .filter(|(_, count)| **count >= 2)
            .map(|(&id, &count)| (id, count))
            .collect();
        heavy.sort_unstable_by_key(|entry| entry.0);
        if let Some(budget) = budget
            && self.prepared_heavy_entries.saturating_add(heavy.len()) > budget.max_heavy_entries
        {
            return None;
        }
        let mut light: Vec<u32> = counts
            .iter()
            .filter(|(_, count)| **count == 1)
            .map(|(&id, _)| id)
            .collect();
        light.sort_unstable();
        if let Some(budget) = budget
            && self.prepared_light_entries.saturating_add(light.len()) > budget.max_light_entries
        {
            return None;
        }

        let mut lengths: Vec<(usize, usize)> = lengths.into_iter().collect();
        lengths.sort_unstable_by_key(|entry| Reverse(entry.0));
        self.prepared_units = self.prepared_units.saturating_add(unit_count);
        self.prepared_light_entries = self.prepared_light_entries.saturating_add(light.len());
        self.prepared_heavy_entries = self.prepared_heavy_entries.saturating_add(heavy.len());
        Some(PreparedSource {
            ids,
            heavy,
            light,
            lengths,
            total_len,
        })
    }

    pub fn unit_bytes(&self, id: u32) -> usize {
        self.unit_bytes[id as usize]
    }
}

/// One file's units, compact frequency classes, and length histogram.
#[derive(Clone, Debug)]
pub struct PreparedSource {
    ids: Vec<u32>,
    // (unit id, count) for units occurring at least twice, sorted by id.
    heavy: Vec<(u32, usize)>,
    // Unit ids occurring exactly once, sorted by id.
    light: Vec<u32>,
    // Unit byte length -> number of units with that length, descending by length.
    lengths: Vec<(usize, usize)>,
    total_len: usize,
}

impl PreparedSource {
    pub fn total_bytes(&self) -> usize {
        self.total_len
    }

    pub fn unit_count(&self) -> usize {
        self.ids.len()
    }

    fn light_contains(&self, id: u32) -> bool {
        self.light.binary_search(&id).is_ok()
    }

    fn light_lengths(
        &self,
        heavy: &HashSet<u32>,
        interner: &SimilarityInterner,
    ) -> Vec<(usize, usize)> {
        let mut lengths = self.lengths.clone();
        for &(id, count) in &self.heavy {
            if !heavy.contains(&id) {
                continue;
            }
            let length = interner.unit_bytes(id);
            if let Some(index) = lengths.iter().position(|entry| entry.0 == length) {
                let remaining = lengths[index].1.saturating_sub(count);
                if remaining == 0 {
                    lengths.remove(index);
                } else {
                    lengths[index].1 = remaining;
                }
            }
        }
        lengths
    }

    fn frequency_upper_bound(&self, other: &Self, interner: &SimilarityInterner) -> usize {
        let mut upper: usize = 0;
        let (small, large) = if self.heavy.len() <= other.heavy.len() {
            (&self.heavy, &other.heavy)
        } else {
            (&other.heavy, &self.heavy)
        };
        for &(id, count) in small {
            if let Ok(index) = large.binary_search_by_key(&id, |entry| entry.0) {
                upper = upper.saturating_add(
                    interner
                        .unit_bytes(id)
                        .saturating_mul(count.min(large[index].1)),
                );
            }
        }
        for &(id, count) in &self.heavy {
            if other.light_contains(id) {
                upper = upper.saturating_add(interner.unit_bytes(id).saturating_mul(count.min(1)));
            }
        }
        for &(id, count) in &other.heavy {
            if self.light_contains(id) {
                upper = upper.saturating_add(interner.unit_bytes(id).saturating_mul(count.min(1)));
            }
        }
        upper.saturating_add(self.light_intersection_bytes(other, interner))
    }

    fn light_intersection_bytes(&self, other: &Self, interner: &SimilarityInterner) -> usize {
        let (mut left, mut right) = (0, 0);
        let mut total: usize = 0;
        while left < self.light.len() && right < other.light.len() {
            match self.light[left].cmp(&other.light[right]) {
                std::cmp::Ordering::Less => left += 1,
                std::cmp::Ordering::Greater => right += 1,
                std::cmp::Ordering::Equal => {
                    total = total.saturating_add(interner.unit_bytes(self.light[left]));
                    left += 1;
                    right += 1;
                }
            }
        }
        total
    }

    fn light_intersection_count(&self, other: &Self) -> usize {
        let (mut left, mut right) = (0, 0);
        let mut total: usize = 0;
        while left < self.light.len() && right < other.light.len() {
            match self.light[left].cmp(&other.light[right]) {
                std::cmp::Ordering::Less => left += 1,
                std::cmp::Ordering::Greater => right += 1,
                std::cmp::Ordering::Equal => {
                    total += 1;
                    left += 1;
                    right += 1;
                }
            }
        }
        total
    }

    fn matching_pairs(&self, other: &Self) -> usize {
        let mut pairs: usize = 0;
        let (small, large) = if self.heavy.len() <= other.heavy.len() {
            (&self.heavy, &other.heavy)
        } else {
            (&other.heavy, &self.heavy)
        };
        for &(id, count) in small {
            if let Ok(index) = large.binary_search_by_key(&id, |entry| entry.0) {
                pairs = pairs.saturating_add(count.saturating_mul(large[index].1));
            }
        }
        for &(id, count) in &self.heavy {
            if other.light_contains(id) {
                pairs = pairs.saturating_add(count);
            }
        }
        for &(id, count) in &other.heavy {
            if self.light_contains(id) {
                pairs = pairs.saturating_add(count);
            }
        }
        pairs.saturating_add(self.light_intersection_count(other))
    }

    // The largest byte total any common subsequence of `count` units can have.
    fn max_bytes_for_units(&self, count: usize) -> usize {
        max_bytes_from_lengths(&self.lengths, count)
    }
}

/// Compute the same thresholded score as [`source_similarity_at_least`] from
/// prepared sources.
///
/// The frequency bound is unchanged. A second safe bound uses the exact LCS
/// unit length, then the largest possible byte total of any common subsequence
/// of that length. Only pairs that pass both bounds reach the deterministic
/// Myers score. The score itself is unchanged for every surviving pair.
pub fn source_similarity_at_least_prepared(
    interner: &SimilarityInterner,
    old: &PreparedSource,
    new: &PreparedSource,
    threshold: u8,
) -> SimilarityOutcome {
    let max = old.total_len.max(new.total_len);
    if max == 0 {
        return SimilarityOutcome::Score(100);
    }

    if !reaches(old.frequency_upper_bound(new, interner), max, threshold) {
        return SimilarityOutcome::BelowThreshold;
    }

    if let Some(heavy_light) = heavy_light_upper_bound(old, new, interner)
        && !reaches(heavy_light, max, threshold)
    {
        return SimilarityOutcome::BelowThreshold;
    }

    if let Some(lcs_units) = lcs_length(old, new) {
        let lcs_bytes = old
            .max_bytes_for_units(lcs_units)
            .min(new.max_bytes_for_units(lcs_units));
        if !reaches(lcs_bytes, max, threshold) {
            return SimilarityOutcome::BelowThreshold;
        }
    }

    let common = common_bytes_prepared(interner, &old.ids, &new.ids);
    let exact = score(common, max);
    if exact >= threshold {
        SimilarityOutcome::Score(exact)
    } else {
        SimilarityOutcome::BelowThreshold
    }
}

fn common_bytes_prepared(interner: &SimilarityInterner, old_ids: &[u32], new_ids: &[u32]) -> usize {
    let operations = capture_diff_slices(Algorithm::Myers, old_ids, new_ids);
    let mut common = 0;
    for operation in operations {
        if let DiffOp::Equal { old_index, len, .. } = operation {
            for &id in &old_ids[old_index..old_index + len] {
                common += interner.unit_bytes(id);
            }
        }
    }
    common
}

// A safe bound for a frequent/rare mixture. Units whose minimum count is at
// least two are "heavy": their byte contribution is at most the frequency
// bound for those units. The remaining "light" units form a common
// subsequence of their filtered sequences, whose exact LCS length is cheap
// when matching pairs are sparse. Heavy bytes plus the largest possible light
// bytes is an upper bound on any common subsequence, including the heuristic
// Myers result.
fn max_bytes_from_lengths(lengths: &[(usize, usize)], count: usize) -> usize {
    let mut remaining = count;
    let mut total = 0;
    for &(length, occurrences) in lengths {
        if remaining == 0 {
            break;
        }
        let take = remaining.min(occurrences);
        total += length * take;
        remaining -= take;
    }
    total
}

fn heavy_light_upper_bound(
    old: &PreparedSource,
    new: &PreparedSource,
    interner: &SimilarityInterner,
) -> Option<usize> {
    let (small, large) = if old.heavy.len() <= new.heavy.len() {
        (&old.heavy, &new.heavy)
    } else {
        (&new.heavy, &old.heavy)
    };
    let mut heavy = HashSet::new();
    let mut heavy_bytes: usize = 0;
    for &(id, count) in small {
        if let Ok(index) = large.binary_search_by_key(&id, |entry| entry.0) {
            let minimum = count.min(large[index].1);
            heavy.insert(id);
            heavy_bytes =
                heavy_bytes.saturating_add(interner.unit_bytes(id).saturating_mul(minimum));
        }
    }

    let mut light_pairs = old.light_intersection_count(new);
    for &(id, count) in &old.heavy {
        if new.light_contains(id) {
            light_pairs = light_pairs.saturating_add(count);
        }
    }
    for &(id, count) in &new.heavy {
        if old.light_contains(id) {
            light_pairs = light_pairs.saturating_add(count);
        }
    }
    if light_pairs > LCS_HUNT_MAX_PAIRS {
        return None;
    }

    let mut positions: HashMap<u32, Vec<usize>> = HashMap::new();
    for (index, &id) in new.ids.iter().enumerate() {
        if !heavy.contains(&id) {
            positions.entry(id).or_default().push(index);
        }
    }
    let light_lcs = lcs_hunt_filtered(&old.ids, &heavy, &positions);
    let old_light_lengths = old.light_lengths(&heavy, interner);
    let new_light_lengths = new.light_lengths(&heavy, interner);
    let light_bytes = max_bytes_from_lengths(&old_light_lengths, light_lcs)
        .min(max_bytes_from_lengths(&new_light_lengths, light_lcs));
    Some(heavy_bytes.saturating_add(light_bytes))
}

fn lcs_hunt_filtered(
    old_ids: &[u32],
    heavy: &HashSet<u32>,
    positions: &HashMap<u32, Vec<usize>>,
) -> usize {
    let mut tails: Vec<usize> = Vec::new();
    for &id in old_ids {
        if heavy.contains(&id) {
            continue;
        }
        if let Some(matches) = positions.get(&id) {
            for &index in matches.iter().rev() {
                let slot = tails.partition_point(|&tail| tail < index);
                if slot == tails.len() {
                    tails.push(index);
                } else {
                    tails[slot] = index;
                }
            }
        }
    }
    tails.len()
}

// Exact LCS length. Hunt-Szymanski is used when matching pairs are sparse;
// bit-parallel LCS is used when the alphabet is dense. Both compute the same
// maximum common unit count, which is an upper bound on the equal units of the
// heuristic Myers score.
fn lcs_length(old: &PreparedSource, new: &PreparedSource) -> Option<usize> {
    let old_len = old.ids.len();
    let new_len = new.ids.len();
    if old_len == 0 || new_len == 0 {
        return Some(0);
    }
    let matching_pairs = old.matching_pairs(new);
    let bit_parallel_work = old_len.saturating_mul(new_len) / 64;
    if matching_pairs <= LCS_HUNT_MAX_PAIRS && matching_pairs <= bit_parallel_work.max(1) {
        return Some(lcs_hunt_szymanski(&old.ids, &new.ids));
    }

    // One full word-mask per distinct new token. The checked estimate is
    // deliberately conservative: masks plus the four working vectors.
    let words = new_len.div_ceil(64);
    let distinct_new = new.heavy.len().saturating_add(new.light.len());
    let scratch = distinct_new
        .saturating_mul(words)
        .saturating_mul(std::mem::size_of::<u64>())
        .saturating_add(words.saturating_mul(std::mem::size_of::<u64>() * 4));
    if scratch > LCS_SCRATCH_BUDGET {
        return None;
    }
    Some(lcs_bit_parallel(&old.ids, &new.ids))
}

fn lcs_hunt_szymanski(old: &[u32], new: &[u32]) -> usize {
    let mut positions: HashMap<u32, Vec<usize>> = HashMap::new();
    for (index, &unit) in new.iter().enumerate() {
        positions.entry(unit).or_default().push(index);
    }

    let mut tails: Vec<usize> = Vec::new();
    for &unit in old {
        if let Some(matches) = positions.get(&unit) {
            for &index in matches.iter().rev() {
                let slot = tails.partition_point(|&tail| tail < index);
                if slot == tails.len() {
                    tails.push(index);
                } else {
                    tails[slot] = index;
                }
            }
        }
    }
    tails.len()
}

fn lcs_bit_parallel(old: &[u32], new: &[u32]) -> usize {
    let words = new.len().div_ceil(64);
    let mut masks: HashMap<u32, Vec<u64>> = HashMap::new();
    for (index, &unit) in new.iter().enumerate() {
        masks.entry(unit).or_insert_with(|| vec![0; words])[index / 64] |= 1u64 << (index % 64);
    }

    let mut v = vec![u64::MAX; words];
    let mut u = vec![0u64; words];
    let mut sum = vec![0u64; words];
    let mut difference = vec![0u64; words];
    for &unit in old {
        if let Some(mask) = masks.get(&unit) {
            for index in 0..words {
                u[index] = v[index] & mask[index];
            }
            add_words(&v, &u, &mut sum);
            sub_words(&v, &u, &mut difference);
            for index in 0..words {
                v[index] = sum[index] | difference[index];
            }
        }
    }

    let mut zeros = 0;
    for index in 0..new.len() {
        if (v[index / 64] >> (index % 64)) & 1 == 0 {
            zeros += 1;
        }
    }
    zeros
}

fn add_words(left: &[u64], right: &[u64], output: &mut [u64]) {
    let mut carry = 0u64;
    for index in 0..left.len() {
        let (sum, first) = left[index].overflowing_add(right[index]);
        let (sum, second) = sum.overflowing_add(carry);
        carry = u64::from(first || second);
        output[index] = sum;
    }
}

fn sub_words(left: &[u64], right: &[u64], output: &mut [u64]) {
    let mut borrow = 0u64;
    for index in 0..left.len() {
        let (difference, first) = left[index].overflowing_sub(right[index]);
        let (difference, second) = difference.overflowing_sub(borrow);
        borrow = u64::from(first || second);
        output[index] = difference;
    }
}

/// The result of a thresholded similarity query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SimilarityOutcome {
    /// The exact score is below the requested threshold; it may not have been
    /// computed because a safe upper bound already ruled it out.
    BelowThreshold,
    /// The exact score, which is at or above the requested threshold.
    Score(u8),
}

/// Compute the exact source-content similarity score, or `None` for non-UTF-8
/// input.
pub fn source_similarity(old: &[u8], new: &[u8]) -> Option<u8> {
    let old = std::str::from_utf8(old).ok()?;
    let new = std::str::from_utf8(new).ok()?;
    let max = old.len().max(new.len());
    if max == 0 {
        return Some(100);
    }

    let (old_units, new_units) = units(old, new);
    let common = common_bytes(&old_units, &new_units);
    Some(score(common, max))
}

/// Compute the score only when it reaches `threshold`, or `None` for non-UTF-8
/// input.
///
/// A pair is rejected without running Myers when a token-frequency upper bound
/// proves that the exact score is below `threshold`. This changes no decision:
/// the upper bound is at least the common byte count of every possible
/// alignment.
pub fn source_similarity_at_least(
    old: &[u8],
    new: &[u8],
    threshold: u8,
) -> Option<SimilarityOutcome> {
    let old = std::str::from_utf8(old).ok()?;
    let new = std::str::from_utf8(new).ok()?;
    let max = old.len().max(new.len());
    if max == 0 {
        return Some(SimilarityOutcome::Score(100));
    }

    let (old_units, new_units) = units(old, new);
    let upper = common_upper_bound(&old_units, &new_units);
    if !reaches(upper, max, threshold) {
        return Some(SimilarityOutcome::BelowThreshold);
    }

    let common = common_bytes(&old_units, &new_units);
    let exact = score(common, max);
    if exact >= threshold {
        Some(SimilarityOutcome::Score(exact))
    } else {
        Some(SimilarityOutcome::BelowThreshold)
    }
}

fn units<'a>(old: &'a str, new: &'a str) -> (Vec<&'a str>, Vec<&'a str>) {
    if old.len() <= SCALAR_UNIT_MAX_BYTES && new.len() <= SCALAR_UNIT_MAX_BYTES {
        (scalar_units(old), scalar_units(new))
    } else {
        (line_units(old), line_units(new))
    }
}

fn scalar_units(source: &str) -> Vec<&str> {
    source
        .char_indices()
        .map(|(index, scalar)| &source[index..index + scalar.len_utf8()])
        .collect()
}

fn line_units(source: &str) -> Vec<&str> {
    source.split_inclusive('\n').collect()
}

fn common_bytes(old_units: &[&str], new_units: &[&str]) -> usize {
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .diff_slices(old_units, new_units);

    let mut common = 0;
    for operation in diff.ops() {
        if let DiffOp::Equal { old_index, len, .. } = *operation {
            for unit in &old_units[old_index..old_index + len] {
                common += unit.len();
            }
        }
    }
    common
}

// An upper bound on `common_bytes`: every equal unit consumes one occurrence
// on each side, so a unit can contribute at most `min(count_old, count_new)`
// copies. Summing the byte length of those copies is therefore at least the
// common byte length of any alignment, including the Myers result.
fn common_upper_bound(old_units: &[&str], new_units: &[&str]) -> usize {
    let mut remaining: HashMap<&str, usize> = HashMap::with_capacity(old_units.len());
    for unit in old_units {
        *remaining.entry(unit).or_default() += 1;
    }

    let mut upper = 0;
    for unit in new_units {
        if let Some(count) = remaining.get_mut(unit)
            && *count > 0
        {
            *count -= 1;
            upper += unit.len();
        }
    }
    upper
}

fn score(common: usize, max: usize) -> u8 {
    ((common as u128 * 100) / max as u128) as u8
}

fn reaches(common: usize, max: usize, threshold: u8) -> bool {
    common as u128 * 100 >= threshold as u128 * max as u128
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_similarity_uses_unicode_scalars() {
        assert_eq!(source_similarity(b"abc", b"axc"), Some(66));
        assert_eq!(source_similarity("é".as_bytes(), "è".as_bytes()), Some(0));
        assert_eq!(source_similarity("é".as_bytes(), "éx".as_bytes()), Some(66));
    }

    #[test]
    fn empty_sources_are_fully_similar() {
        assert_eq!(source_similarity(b"", b""), Some(100));
        assert_eq!(
            source_similarity_at_least(b"", b"", 50),
            Some(SimilarityOutcome::Score(100))
        );
    }

    #[test]
    fn invalid_utf8_has_no_score() {
        assert_eq!(source_similarity(b"a\xFF", b"a"), None);
        assert_eq!(source_similarity_at_least(b"a\xFF", b"a", 50), None);
    }

    #[test]
    fn large_sources_use_complete_lines_including_endings() {
        let old = "line\r\n".repeat(1_000);
        let new = "line\n".repeat(1_000);
        assert!(old.len() > SCALAR_UNIT_MAX_BYTES);
        assert_eq!(
            source_similarity(old.as_bytes(), new.as_bytes()),
            Some(0),
            "line endings are source content and must not be normalized"
        );

        let mut changed = old.clone();
        changed.replace_range(0.."line\r\n".len(), "other\r\n");
        assert_eq!(
            source_similarity(old.as_bytes(), changed.as_bytes()),
            Some(99),
            "one changed line out of 1,000 leaves 999 * 6 common bytes"
        );
    }

    #[test]
    fn either_side_over_four_kib_selects_line_units() {
        let small = "a".repeat(SCALAR_UNIT_MAX_BYTES);
        let large = format!("{small}\n");
        assert_eq!(
            source_similarity(small.as_bytes(), large.as_bytes()),
            Some(0),
            "the 4 KiB side is compared as a complete line when the other side is larger; \
             scalar units would score 99"
        );
    }

    #[test]
    fn thresholded_similarity_matches_the_exact_score() {
        let cases: &[(&[u8], &[u8])] = &[
            (b"", b""),
            (b"abc", b"axc"),
            (b"abc", b"xyz"),
            (b"same\n", b"same\n"),
            (b"a\nb\n", b"a\nc\n"),
        ];
        for (old, new) in cases {
            for threshold in [0, 1, 50, 66, 67, 100] {
                let exact = source_similarity(old, new).expect("valid UTF-8");
                let thresholded =
                    source_similarity_at_least(old, new, threshold).expect("valid UTF-8");
                let expected = if exact >= threshold {
                    SimilarityOutcome::Score(exact)
                } else {
                    SimilarityOutcome::BelowThreshold
                };
                assert_eq!(
                    thresholded, expected,
                    "old={old:?} new={new:?} threshold={threshold}"
                );
            }
        }
    }

    #[test]
    fn a_disjoint_large_pair_is_pruned_by_the_frequency_bound() {
        let old = "alpha\n".repeat(1_000);
        let new = "bravo\n".repeat(1_000);
        assert_eq!(
            source_similarity_at_least(old.as_bytes(), new.as_bytes(), 50),
            Some(SimilarityOutcome::BelowThreshold)
        );
    }

    #[test]
    fn the_frequency_bound_is_byte_weighted_not_token_counted() {
        let long_line = format!("// {}\n", "shared".repeat(1_000));
        let old = format!("{long_line}{}", "a\n".repeat(1_000));
        let new = format!("{long_line}{}", "b\n".repeat(1_000));
        assert!(old.len() > SCALAR_UNIT_MAX_BYTES);
        let score = source_similarity(old.as_bytes(), new.as_bytes()).expect("UTF-8");
        assert!(score >= 50, "expected a byte-weighted score, got {score}");
        assert_eq!(
            source_similarity_at_least(old.as_bytes(), new.as_bytes(), 50),
            Some(SimilarityOutcome::Score(score)),
            "a token-count bound would falsely prune the long shared line"
        );
    }

    #[test]
    fn the_frequency_bound_weights_mixed_width_scalars_by_utf8_bytes() {
        let old = "aé☃".repeat(500);
        let new = format!("{}b", "aé☃".repeat(499));
        let score = source_similarity(old.as_bytes(), new.as_bytes()).expect("UTF-8");
        assert!(score >= 50);
        assert_eq!(
            source_similarity_at_least(old.as_bytes(), new.as_bytes(), 50),
            Some(SimilarityOutcome::Score(score))
        );
    }

    #[test]
    fn a_shared_final_line_without_a_newline_is_one_complete_unit() {
        let shared = "x".repeat(SCALAR_UNIT_MAX_BYTES + 1);
        let old = format!("header\n{shared}");
        let new = format!("header\nextra\n{shared}");
        assert_eq!(
            source_similarity(old.as_bytes(), new.as_bytes()),
            Some(99),
            "the final unterminated line must be compared as a complete line"
        );
    }

    #[test]
    fn the_frequency_bound_does_not_replace_the_exact_myers_score() {
        let old = "abcdefghij".repeat(100);
        let new = "jihgfedcba".repeat(100);
        let exact = source_similarity(old.as_bytes(), new.as_bytes()).expect("UTF-8");
        assert!(exact < 50, "expected a low exact score, got {exact}");
        assert_eq!(
            source_similarity_at_least(old.as_bytes(), new.as_bytes(), 50),
            Some(SimilarityOutcome::BelowThreshold),
            "the same multiset must still be scored exactly"
        );
    }

    #[test]
    fn shuffled_equal_multisets_are_not_pruned_by_the_bound() {
        let old = format!("{}{}", "a\n".repeat(1_000), "b\n".repeat(1_000));
        let new = format!("{}{}", "b\n".repeat(1_000), "a\n".repeat(1_000));
        assert_eq!(
            source_similarity_at_least(old.as_bytes(), new.as_bytes(), 50),
            Some(SimilarityOutcome::Score(50)),
            "the bound must leave an equal multiset for exact Myers scoring"
        );
    }

    fn lcs_dp(old: &[u32], new: &[u32]) -> usize {
        let mut previous = vec![0usize; new.len() + 1];
        for &old_unit in old {
            let mut current = vec![0usize; new.len() + 1];
            for (index, &new_unit) in new.iter().enumerate() {
                current[index + 1] = if old_unit == new_unit {
                    previous[index] + 1
                } else {
                    previous[index + 1].max(current[index])
                };
            }
            previous = current;
        }
        previous[new.len()]
    }

    #[test]
    fn lcs_algorithms_agree_with_dynamic_programming() {
        let mut state = 0x1234_5678_9abc_def0u64;
        let mut random = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..2_000 {
            let old_len = (random() % 30) as usize;
            let new_len = (random() % 30) as usize;
            let old: Vec<u32> = (0..old_len).map(|_| (random() % 6) as u32).collect();
            let new: Vec<u32> = (0..new_len).map(|_| (random() % 6) as u32).collect();
            let expected = lcs_dp(&old, &new);
            assert_eq!(lcs_hunt_szymanski(&old, &new), expected);
            assert_eq!(lcs_bit_parallel(&old, &new), expected);
        }
    }

    fn random_scalar_source(random: &mut impl FnMut() -> u64) -> String {
        let alphabet = ['a', 'b', 'c', 'é', '☃', '\n', '\r', '\t'];
        let len = (random() % 700) as usize;
        (0..len)
            .map(|_| alphabet[(random() as usize) % alphabet.len()])
            .collect()
    }

    fn random_line_source(random: &mut impl FnMut() -> u64, count: usize) -> String {
        let alphabet = ['a', 'b', 'c', 'é', '\r'];
        let mut source = String::new();
        for _ in 0..count {
            let len = 1 + (random() % 24) as usize;
            for _ in 0..len {
                source.push(alphabet[(random() as usize) % alphabet.len()]);
            }
            source.push('\n');
        }
        source
    }

    fn random_wide_line_source(random: &mut impl FnMut() -> u64, count: usize) -> String {
        let alphabet = ['a', 'b', 'c', 'é', '\r'];
        let mut source = String::new();
        for _ in 0..count {
            let len = 50 + (random() % 30) as usize;
            for _ in 0..len {
                source.push(alphabet[(random() as usize) % alphabet.len()]);
            }
            source.push('\n');
        }
        source
    }

    #[test]
    fn prepared_similarity_prunes_reversed_multisets_and_keeps_a_50_percent_move() {
        let reversed_old = "abcdefghij".repeat(380);
        let reversed_new = "jihgfedcba".repeat(380);
        let mut interner = SimilarityInterner::new();
        let old = interner.prepare(&reversed_old, UnitMode::Scalar);
        let new = interner.prepare(&reversed_new, UnitMode::Scalar);
        assert_eq!(
            source_similarity_at_least_prepared(&interner, &old, &new, 50),
            SimilarityOutcome::BelowThreshold
        );
        assert!(
            source_similarity(reversed_old.as_bytes(), reversed_new.as_bytes())
                .expect("valid UTF-8")
                < 50
        );

        let shared = (0..1_000)
            .map(|index| format!("// shared_{index:04}\n"))
            .collect::<String>();
        let old_unique = (0..1_000)
            .map(|index| format!("// old____{index:04}\n"))
            .collect::<String>();
        let new_unique = (0..1_000)
            .map(|index| format!("// new____{index:04}\n"))
            .collect::<String>();
        let old_text = format!("{shared}{old_unique}");
        let new_text = format!("{shared}{new_unique}");
        let old = interner.prepare(&old_text, UnitMode::Line);
        let new = interner.prepare(&new_text, UnitMode::Line);
        let exact = source_similarity(old_text.as_bytes(), new_text.as_bytes()).expect("UTF-8");
        assert_eq!(exact, 50);
        assert_eq!(
            source_similarity_at_least_prepared(&interner, &old, &new, 50),
            SimilarityOutcome::Score(50)
        );
    }

    #[test]
    fn profile_budget_rejects_further_preparation_without_changing_the_scorer() {
        let mut interner = SimilarityInterner::new();
        let budget = ProfileBudget {
            max_units: 2,
            max_light_entries: 2,
            max_heavy_entries: 2,
            max_interned_bytes: 1_024,
        };
        assert!(
            interner
                .try_prepare("ab", UnitMode::Scalar, budget)
                .is_some()
        );
        assert!(
            interner
                .try_prepare("cd", UnitMode::Scalar, budget)
                .is_none(),
            "the unit budget must stop further prepared profiles"
        );

        let mut heavy_interner = SimilarityInterner::new();
        let heavy_budget = ProfileBudget {
            max_units: 10,
            max_light_entries: 10,
            max_heavy_entries: 0,
            max_interned_bytes: 1_024,
        };
        assert!(
            heavy_interner
                .try_prepare("aa", UnitMode::Scalar, heavy_budget)
                .is_none(),
            "the heavy-entry budget must stop repeated-token profiles"
        );

        let mut cumulative = SimilarityInterner::new();
        let cumulative_budget = ProfileBudget {
            max_units: 10,
            max_light_entries: 10,
            max_heavy_entries: 1,
            max_interned_bytes: 1_024,
        };
        assert!(
            cumulative
                .try_prepare("aa", UnitMode::Scalar, cumulative_budget)
                .is_some(),
            "the first heavy profile fits the cumulative budget"
        );
        assert!(
            cumulative
                .try_prepare("bb", UnitMode::Scalar, cumulative_budget)
                .is_none(),
            "the second heavy profile must see the first profile's heavy entry"
        );
        assert_eq!(
            source_similarity_at_least(b"aa", b"bb", 50),
            Some(SimilarityOutcome::BelowThreshold),
            "the budget rejection must not change the public score"
        );

        // The public scorer is independent of the optimization budget.
        let old = interner.prepare("ab", UnitMode::Scalar);
        let new = interner.prepare("cd", UnitMode::Scalar);
        assert_eq!(
            source_similarity_at_least_prepared(&interner, &old, &new, 50),
            SimilarityOutcome::BelowThreshold
        );
        assert_eq!(
            source_similarity_at_least(b"ab", b"cd", 50),
            Some(SimilarityOutcome::BelowThreshold)
        );
    }

    #[test]
    fn heavy_light_bound_prunes_all_unique_mixed_sources() {
        let alphabet = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        let mut comments = Vec::new();
        for index in 0..38_000usize {
            let mut value = index;
            let mut name = [0u8; 3];
            for slot in (0..3).rev() {
                name[slot] = alphabet[value % alphabet.len()];
                value /= alphabet.len();
            }
            comments.push(format!(
                "//{}{}{}\n",
                name[0] as char, name[1] as char, name[2] as char
            ));
        }
        let blanks = "\n".repeat(20_000);
        let old = format!("{}{blanks}", comments.concat());
        let new = format!(
            "{blanks}{}",
            comments.iter().rev().cloned().collect::<String>()
        );
        assert!(old.len() <= 256 * 1024);
        assert!(new.len() <= 256 * 1024);

        let mut interner = SimilarityInterner::new();
        let old_prepared = interner.prepare(&old, UnitMode::Line);
        let new_prepared = interner.prepare(&new, UnitMode::Line);
        let bound = heavy_light_upper_bound(&old_prepared, &new_prepared, &interner)
            .expect("the light match set is sparse");
        assert!(
            bound * 100 < old.len().max(new.len()) * 50,
            "the heavy/light bound must prune the unique mixture"
        );
        assert_eq!(
            source_similarity_at_least_prepared(&interner, &old_prepared, &new_prepared, 50),
            SimilarityOutcome::BelowThreshold
        );
        assert!(
            source_similarity_at_least(old.as_bytes(), new.as_bytes(), 50).expect("valid UTF-8")
                == SimilarityOutcome::BelowThreshold
        );
    }

    #[test]
    fn mixed_alphabet_lcs_budget_falls_back_without_changing_the_score() {
        let comments = (0..10_000)
            .map(|index| format!("# comment_{index:05}\n"))
            .collect::<String>();
        let blanks = "\n".repeat(100_000);
        let old = format!("x = 1\n{comments}{blanks}");
        let new = format!(
            "x = 1\n{blanks}{}",
            (0..10_000)
                .rev()
                .map(|index| format!("# comment_{index:05}\n"))
                .collect::<String>()
        );
        assert!(old.len() <= 256 * 1024);
        assert!(new.len() <= 256 * 1024);

        let mut interner = SimilarityInterner::new();
        let old_prepared = interner.prepare(&old, UnitMode::Line);
        let new_prepared = interner.prepare(&new, UnitMode::Line);
        assert_eq!(
            lcs_length(&old_prepared, &new_prepared),
            None,
            "the mixed alphabet must exceed the LCS scratch budget"
        );

        for threshold in [50, 100] {
            let expected = source_similarity_at_least(old.as_bytes(), new.as_bytes(), threshold)
                .expect("valid UTF-8");
            let actual = source_similarity_at_least_prepared(
                &interner,
                &old_prepared,
                &new_prepared,
                threshold,
            );
            assert_eq!(
                actual, expected,
                "skipping the LCS bound must not change the threshold decision"
            );
        }
    }

    #[test]
    fn prepared_similarity_matches_thresholds_on_small_cases() {
        let cases: &[(&str, &str)] = &[
            ("", ""),
            ("abc", "axc"),
            ("abc", "xyz"),
            ("same\n", "same\n"),
            ("a\nb\n", "a\nc\n"),
            ("é☃a", "é☃b"),
        ];
        let mut interner = SimilarityInterner::new();
        for (old, new) in cases {
            let old_prepared = interner.prepare(old, UnitMode::Scalar);
            let new_prepared = interner.prepare(new, UnitMode::Scalar);
            for threshold in [0, 1, 50, 66, 67, 100] {
                let exact = source_similarity(old.as_bytes(), new.as_bytes()).expect("valid UTF-8");
                let expected = if exact >= threshold {
                    SimilarityOutcome::Score(exact)
                } else {
                    SimilarityOutcome::BelowThreshold
                };
                assert_eq!(
                    source_similarity_at_least_prepared(
                        &interner,
                        &old_prepared,
                        &new_prepared,
                        threshold
                    ),
                    expected
                );
            }
        }
    }

    #[test]
    fn prepared_similarity_matches_exhaustive_small_sequences() {
        let mut sequences = vec![String::new()];
        for length in 1..=4 {
            let previous = sequences
                .iter()
                .filter(|sequence| sequence.chars().count() == length - 1)
                .cloned()
                .collect::<Vec<_>>();
            for sequence in previous {
                for token in ['a', 'b', 'c'] {
                    let mut next = sequence.clone();
                    next.push(token);
                    sequences.push(next);
                }
            }
        }

        let mut interner = SimilarityInterner::new();
        for old in &sequences {
            for new in &sequences {
                let old_prepared = interner.prepare(old, UnitMode::Scalar);
                let new_prepared = interner.prepare(new, UnitMode::Scalar);
                let exact = source_similarity(old.as_bytes(), new.as_bytes()).expect("valid UTF-8");
                for threshold in [1, 50, 100] {
                    let expected = if exact >= threshold {
                        SimilarityOutcome::Score(exact)
                    } else {
                        SimilarityOutcome::BelowThreshold
                    };
                    assert_eq!(
                        source_similarity_at_least_prepared(
                            &interner,
                            &old_prepared,
                            &new_prepared,
                            threshold
                        ),
                        expected,
                        "old={old:?} new={new:?} threshold={threshold}"
                    );
                }
            }
        }
    }

    #[test]
    fn prepared_similarity_matches_the_unchanged_public_score() {
        let mut state = 0x0f1e_2d3c_4b5a_6978u64;
        let mut random = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut interner = SimilarityInterner::new();

        for _ in 0..100 {
            let (old, new) = match random() % 4 {
                0 => (
                    random_scalar_source(&mut random),
                    random_scalar_source(&mut random),
                ),
                1 => {
                    let old_count = (random() % 250) as usize;
                    let new_count = (random() % 250) as usize;
                    (
                        random_line_source(&mut random, old_count),
                        random_line_source(&mut random, new_count),
                    )
                }
                2 => (
                    random_wide_line_source(&mut random, 100),
                    random_wide_line_source(&mut random, 100),
                ),
                _ => (
                    random_scalar_source(&mut random),
                    random_wide_line_source(&mut random, 100),
                ),
            };
            let scalar = old.len() <= SCALAR_UNIT_MAX_BYTES && new.len() <= SCALAR_UNIT_MAX_BYTES;
            let mode = if scalar {
                UnitMode::Scalar
            } else {
                UnitMode::Line
            };
            let old_prepared = interner.prepare(&old, mode);
            let new_prepared = interner.prepare(&new, mode);

            for threshold in [0, 1, 49, 50, 51, 100] {
                let expected =
                    source_similarity_at_least(old.as_bytes(), new.as_bytes(), threshold)
                        .expect("valid UTF-8");
                let actual = source_similarity_at_least_prepared(
                    &interner,
                    &old_prepared,
                    &new_prepared,
                    threshold,
                );
                assert_eq!(
                    actual, expected,
                    "prepared score changed for old={old:?} new={new:?} threshold={threshold}"
                );
                match actual {
                    SimilarityOutcome::Score(score) => {
                        assert_eq!(
                            source_similarity(old.as_bytes(), new.as_bytes()),
                            Some(score),
                            "surviving score must remain exact"
                        );
                    }
                    SimilarityOutcome::BelowThreshold => {
                        assert!(
                            source_similarity(old.as_bytes(), new.as_bytes()).expect("valid UTF-8")
                                < threshold
                        );
                    }
                }
            }
        }
    }
}
