//! Bijection of actual capabilities to the closed expected SID set.
pub(super) fn exact(masks: &[u8]) -> bool {
    if masks.is_empty() || masks.len() > 2 {
        return false;
    }
    let full = (1u8 << masks.len()) - 1;
    let mut seen = 0;
    for mask in masks {
        if mask.count_ones() != 1 || mask & !full != 0 || seen & mask != 0 {
            return false;
        }
        seen |= mask;
    }
    seen == full
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn singleton_and_exact_pair_allow_permutation_but_never_duplicates_or_unknown_sids() {
        for good in [&[1][..], &[1, 2], &[2, 1]] {
            assert!(exact(good));
        }
        for bad in [
            &[][..],
            &[0],
            &[2],
            &[1, 1],
            &[2, 2],
            &[3, 0],
            &[1, 4],
            &[1, 2, 4],
        ] {
            assert!(!exact(bad));
        }
    }
}
