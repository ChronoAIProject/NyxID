//! Shared queue/scheduler helpers for worker-pull relay features.
//!
//! This module intentionally contains only pure scheduling primitives.
//! Compute pools, Oracle pools, MCP tools, and future relays keep their own
//! task documents, payloads, ACLs, and worker protocols. Shared logic belongs
//! here only when it is independent of the business payload.

use chrono::{DateTime, Utc};
use mongodb::bson::{Document, doc};

pub const WILDCARD_LABEL: &str = "*";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum QueueOrdering {
    Fifo,
    PriorityFifo,
}

pub fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect()
    }
}

pub fn normalize_labels(labels: &[String], max_count: usize, max_len: usize) -> Vec<String> {
    let mut out = Vec::new();
    for label in labels.iter().take(max_count) {
        let label = label.trim();
        if label.is_empty() {
            continue;
        }
        let label = truncate_chars(label, max_len);
        if !out.contains(&label) {
            out.push(label);
        }
    }
    out
}

pub fn labels_accept_any(labels: &[String]) -> bool {
    labels.iter().any(|label| label == WILDCARD_LABEL)
}

/// Build a Mongo `$in` filter for a required label.
///
/// `None` means the worker explicitly accepts any label. An empty `$in` means
/// the worker advertised no labels and should not claim label-scoped work.
pub fn required_label_filter(advertised_labels: &[String]) -> Option<Document> {
    if labels_accept_any(advertised_labels) {
        None
    } else {
        Some(doc! { "$in": advertised_labels })
    }
}

pub fn claim_sort(ordering: QueueOrdering) -> Document {
    match ordering {
        QueueOrdering::Fifo => doc! { "created_at": 1 },
        QueueOrdering::PriorityFifo => doc! { "priority": -1, "created_at": 1 },
    }
}

pub fn queue_position_ahead_filter(
    ordering: QueueOrdering,
    priority: i32,
    created_at: DateTime<Utc>,
) -> Document {
    match ordering {
        QueueOrdering::Fifo => {
            doc! { "created_at": { "$lt": bson::DateTime::from_chrono(created_at) } }
        }
        QueueOrdering::PriorityFifo => doc! {
            "$or": [
                { "priority": { "$gt": priority } },
                {
                    "priority": priority,
                    "created_at": { "$lt": bson::DateTime::from_chrono(created_at) },
                },
            ],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_labels_trims_truncates_deduplicates_and_caps() {
        let labels = vec![
            " codex-local ".to_string(),
            "codex-local".to_string(),
            "averyverylonglabel".to_string(),
            "".to_string(),
            "ignored-after-cap".to_string(),
        ];

        assert_eq!(
            normalize_labels(&labels, 3, 8),
            vec!["codex-lo".to_string(), "averyver".to_string()]
        );
    }

    #[test]
    fn label_filter_requires_explicit_wildcard_for_any() {
        assert_eq!(required_label_filter(&["*".to_string()]), None);
        assert_eq!(
            required_label_filter(&["codex-local".to_string()]),
            Some(doc! { "$in": ["codex-local"] })
        );
        assert_eq!(
            required_label_filter(&[]),
            Some(doc! { "$in": Vec::<String>::new() })
        );
    }

    #[test]
    fn claim_sort_documents_are_stable() {
        assert_eq!(claim_sort(QueueOrdering::Fifo), doc! { "created_at": 1 });
        assert_eq!(
            claim_sort(QueueOrdering::PriorityFifo),
            doc! { "priority": -1, "created_at": 1 }
        );
    }

    #[test]
    fn queue_position_filters_match_ordering() {
        let created_at = Utc::now();
        assert_eq!(
            queue_position_ahead_filter(QueueOrdering::Fifo, 7, created_at),
            doc! { "created_at": { "$lt": bson::DateTime::from_chrono(created_at) } }
        );
        assert_eq!(
            queue_position_ahead_filter(QueueOrdering::PriorityFifo, 7, created_at),
            doc! {
                "$or": [
                    { "priority": { "$gt": 7 } },
                    {
                        "priority": 7,
                        "created_at": { "$lt": bson::DateTime::from_chrono(created_at) },
                    },
                ],
            }
        );
    }
}
