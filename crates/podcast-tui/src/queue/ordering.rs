use crate::models::QueueItem;

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
