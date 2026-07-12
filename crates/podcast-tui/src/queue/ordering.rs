use crate::models::QueueItem;
use uuid::Uuid;

/// Find the nearest legal insert position for an episode of podcast `sub` into a
/// queue whose items belong to `subs` (in order). "Legal" means the insertion
/// does not put two episodes of the same podcast adjacent. Positions below
/// `floor` are off-limits (they protect the currently-playing head). Searches
/// upward from `start` when `to_top`, downward otherwise; falls back to `start`
/// if nothing is legal.
pub fn nearest_legal_position(
    subs: &[Uuid],
    sub: Uuid,
    start: usize,
    floor: usize,
    to_top: bool,
) -> usize {
    let len = subs.len();
    let legal = |p: usize| {
        let left_ok = p == 0 || subs[p - 1] != sub;
        let right_ok = p >= len || subs[p] != sub;
        p >= floor && left_ok && right_ok
    };
    if to_top {
        (start..=len).find(|&p| legal(p)).unwrap_or(start)
    } else {
        (floor..=start.min(len))
            .rev()
            .find(|&p| legal(p))
            .unwrap_or(start)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum QueueOrdering {
    Manual,
    DateAscending,
    DateDescending,
    Priority,
}

impl QueueOrdering {
    pub fn sort(&self, items: &mut [QueueItem]) {
        match self {
            QueueOrdering::Manual => {
                // Already sorted by position
                items.sort_by_key(|item| item.position);
            }
            QueueOrdering::DateAscending => {
                items.sort_by_key(|item| item.added_at);
            }
            QueueOrdering::DateDescending => {
                items.sort_by_key(|item| std::cmp::Reverse(item.added_at));
            }
            QueueOrdering::Priority => {
                items.sort_by(|a, b| {
                    use crate::models::QueuePriority;
                    let priority_value = |p: &QueuePriority| match p {
                        QueuePriority::High => 0,
                        QueuePriority::Medium => 1,
                        QueuePriority::Low => 2,
                    };
                    priority_value(&a.priority).cmp(&priority_value(&b.priority))
                });
            }
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Manual => "Manual",
            Self::DateAscending => "Date (Oldest First)",
            Self::DateDescending => "Date (Newest First)",
            Self::Priority => "Priority",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::nearest_legal_position;
    use uuid::Uuid;

    #[test]
    fn empty_queue_takes_the_start_position() {
        let a = Uuid::new_v4();
        assert_eq!(nearest_legal_position(&[], a, 0, 0, true), 0);
        assert_eq!(nearest_legal_position(&[], a, 0, 0, false), 0);
    }

    #[test]
    fn push_backs_off_from_a_same_podcast_tail() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        // Queue: [a, a, b]; pushing another `b` at the end (pos 3) is illegal
        // (next to the trailing `b`), pos 2 is too (right neighbor `b`), so it
        // backs off to pos 1 -> [a, b, a, b], adjacency-free.
        let subs = [a, a, b];
        assert_eq!(nearest_legal_position(&subs, b, 3, 0, false), 1);
    }

    #[test]
    fn push_at_tail_is_legal_when_different() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let subs = [a, b];
        // Pushing `a` at the end sits next to `b`: legal, stays at pos 2.
        assert_eq!(nearest_legal_position(&subs, a, 2, 0, false), 2);
    }

    #[test]
    fn unshift_skips_forward_past_same_podcast() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        // Queue: [a, b]; unshifting `a` at the top (pos 0) is illegal, so it
        // moves to pos 1 (between a and b) - still adjacent to a on the left...
        // pos 1 has left=a (same) so illegal; pos 2 has left=b, legal.
        let subs = [a, b];
        assert_eq!(nearest_legal_position(&subs, a, 0, 0, true), 2);
    }

    #[test]
    fn floor_protects_the_current_head() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        // Head `a` is protected (floor 1). Unshifting `b`: pos 1 has left=a,
        // right=b -> right is same, illegal; pos 2 has left=b... wait subs=[a,b]
        // pos 2 left=b same -> illegal; falls back to start (1). Use a cleaner
        // case: queue [a, a], unshift b with floor 1 -> pos 1 legal (left a,
        // right a, neither is b).
        let subs = [a, a];
        let pos = nearest_legal_position(&subs, b, 1, 1, true);
        assert!(pos >= 1, "never inserts before the protected head");
        assert_eq!(pos, 1);
    }
}
