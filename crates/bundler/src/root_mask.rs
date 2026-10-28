//! Immutable plan-local root sets; scalar membership never owns root payloads.
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::chunks::RootId;

/// Canonically ordered roots sharing a dictionary and immutable scalar membership.
/// Equality, ordering and hashing compare root values, even across dictionaries.
#[derive(Clone, Default)]
pub struct RootMask {
    roots: Arc<[RootId]>,
    indices: Arc<[usize]>,
}

impl RootMask {
    pub(super) fn new(roots: Arc<[RootId]>, indices: Arc<[usize]>) -> Self {
        Self { roots, indices }
    }

    pub fn iter(&self) -> RootMaskIter<'_> {
        self.into_iter()
    }

    pub fn contains(&self, root: &RootId) -> bool {
        self.roots
            .binary_search(root)
            .is_ok_and(|index| self.indices.binary_search(&index).is_ok())
    }

    pub fn len(&self) -> usize {
        self.indices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Explicit owned compatibility projection, not retained planner storage.
    pub fn to_owned_set(&self) -> BTreeSet<RootId> {
        self.iter().cloned().collect()
    }

    pub fn is_subset(&self, other: &Self) -> bool {
        self.iter().all(|root| other.contains(root))
    }

    pub fn is_disjoint(&self, other: &Self) -> bool {
        !self.iter().any(|root| other.contains(root))
    }

    pub fn intersection<'a>(&'a self, other: &'a Self) -> impl Iterator<Item = &'a RootId> {
        self.iter().filter(|root| other.contains(root))
    }

    pub fn union<'a>(&'a self, other: &'a Self) -> impl Iterator<Item = &'a RootId> {
        let mut left = self.iter().peekable();
        let mut right = other.iter().peekable();
        std::iter::from_fn(move || match (left.peek(), right.peek()) {
            (Some(a), Some(b)) => match a.cmp(b) {
                std::cmp::Ordering::Less => left.next(),
                std::cmp::Ordering::Greater => right.next(),
                std::cmp::Ordering::Equal => {
                    right.next();
                    left.next()
                }
            },
            (Some(_), None) => left.next(),
            (None, _) => right.next(),
        })
    }
}

impl From<BTreeSet<RootId>> for RootMask {
    fn from(roots: BTreeSet<RootId>) -> Self {
        Self {
            indices: (0..roots.len()).collect(),
            roots: roots.into_iter().collect(),
        }
    }
}

impl FromIterator<RootId> for RootMask {
    fn from_iter<T: IntoIterator<Item = RootId>>(roots: T) -> Self {
        BTreeSet::from_iter(roots).into()
    }
}

impl PartialEq for RootMask {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.roots, &other.roots) {
            self.indices == other.indices
        } else {
            self.iter().eq(other.iter())
        }
    }
}

impl Eq for RootMask {}

impl PartialOrd for RootMask {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RootMask {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if Arc::ptr_eq(&self.roots, &other.roots) {
            self.indices.cmp(&other.indices)
        } else {
            self.iter().cmp(other.iter())
        }
    }
}

impl Hash for RootMask {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.len().hash(state);
        for root in self {
            root.hash(state);
        }
    }
}

impl PartialEq<BTreeSet<RootId>> for RootMask {
    fn eq(&self, other: &BTreeSet<RootId>) -> bool {
        self.iter().eq(other.iter())
    }
}

impl PartialEq<RootMask> for BTreeSet<RootId> {
    fn eq(&self, other: &RootMask) -> bool {
        other == self
    }
}

impl std::fmt::Debug for RootMask {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_set().entries(self).finish()
    }
}

impl<'a> IntoIterator for &'a RootMask {
    type Item = &'a RootId;
    type IntoIter = RootMaskIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        RootMaskIter {
            roots: &self.roots,
            indices: self.indices.iter(),
        }
    }
}

pub struct RootMaskIter<'a> {
    roots: &'a [RootId],
    indices: std::slice::Iter<'a, usize>,
}

impl<'a> Iterator for RootMaskIter<'a> {
    type Item = &'a RootId;

    fn next(&mut self) -> Option<Self::Item> {
        self.indices.next().map(|&index| &self.roots[index])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.indices.size_hint()
    }
}

impl DoubleEndedIterator for RootMaskIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.indices.next_back().map(|&index| &self.roots[index])
    }
}

impl ExactSizeIterator for RootMaskIter<'_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunks::AsyncSiteId;

    fn hash(mask: &RootMask) -> u64 {
        let mut state = std::collections::hash_map::DefaultHasher::new();
        mask.hash(&mut state);
        state.finish()
    }

    #[test]
    fn canonical_values_compare_across_dictionaries_and_empty_masks() {
        let a = RootId::Entry("a".into());
        let b = RootId::Entry("b".into());
        let c = RootId::Entry("c".into());
        let dictionary: Arc<[RootId]> =
            vec![RootId::Entry("0".into()), a.clone(), b.clone(), c.clone()].into();
        let shared = RootMask::new(dictionary.clone(), vec![1, 2].into());
        let standalone: RootMask = [b.clone(), a.clone(), a.clone()].into_iter().collect();
        let other = RootMask::new(dictionary.clone(), vec![2, 3].into());
        assert_eq!(shared, standalone);
        assert_eq!(shared.cmp(&standalone), std::cmp::Ordering::Equal);
        assert_eq!(hash(&shared), hash(&standalone));
        assert_eq!(
            BTreeSet::from([shared.clone(), standalone.clone()]).len(),
            1
        );
        assert_eq!(shared.len(), 2);
        assert!(shared.iter().eq(standalone.iter()));
        assert!(shared.iter().rev().eq(standalone.iter().rev()));
        assert_eq!(shared.iter().len(), 2);
        assert!(shared < other);
        assert!(standalone < other);
        assert!(!shared.contains(&c));
        assert_eq!(
            shared.union(&other).cloned().collect::<Vec<_>>(),
            [a, b.clone(), c]
        );
        assert_eq!(shared.intersection(&other).collect::<Vec<_>>(), [&b]);
        assert!(shared.is_subset(&standalone));
        assert!(!shared.is_disjoint(&other));
        let empty = RootMask::new(dictionary, Arc::default());
        assert_eq!(empty, RootMask::default());
        assert_eq!(hash(&empty), hash(&RootMask::default()));
        assert!(empty.iter().next().is_none());
        assert!(empty.iter().next_back().is_none());
        assert!(empty.is_subset(&shared));
        assert!(empty.is_disjoint(&shared));
        assert_eq!(
            format!("{shared:?}"),
            format!("{:?}", shared.to_owned_set())
        );
    }

    #[test]
    fn full_async_payloads_and_context_candidates_remain_root_identity() {
        let site = AsyncSiteId {
            module: "long-source-component\\".repeat(16).into(),
            offset: 42,
            candidate: Some("candidate-a.mjs".into()),
        };
        let root = RootId::AsyncExport {
            site: site.clone(),
            name: "value".into(),
        };
        let mut variants = vec![root.clone(), RootId::Async(site.clone())];
        for variant in 0..4 {
            let mut changed_site = site.clone();
            let mut name = "value".to_owned();
            match variant {
                0 => changed_site.module = "different-source.mjs".into(),
                1 => changed_site.offset += 1,
                2 => changed_site.candidate = Some("candidate-b.mjs".into()),
                _ => name = "other".into(),
            }
            variants.push(RootId::AsyncExport {
                site: changed_site,
                name,
            });
        }
        let mask: RootMask = variants.clone().into_iter().collect();
        assert_eq!(mask.len(), variants.len());
        let only: RootMask = [root.clone()].into_iter().collect();
        assert!(only.is_subset(&mask));
        assert!(mask.contains(&root));
        for variant in &variants[1..] {
            assert!(!only.contains(variant));
            let other: RootMask = [variant.clone()].into_iter().collect();
            assert_ne!(only, other);
            assert_eq!(only.cmp(&other), root.cmp(variant));
            assert!(only.is_disjoint(&other));
        }
        let no_candidate = RootId::AsyncExport {
            site: AsyncSiteId {
                candidate: None,
                ..site
            },
            name: "value".into(),
        };
        assert!(!mask.contains(&no_candidate));
    }
}
