pub(super) fn number(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(digit);
    }
    output
}

pub(super) fn counted(value: u64, singular: &str, plural: &str) -> String {
    format!(
        "{} {}",
        number(value),
        if value == 1 { singular } else { plural }
    )
}

pub(super) fn oldest<'a>(ages: impl Iterator<Item = &'a str>) -> String {
    let ages: std::collections::BTreeSet<_> = ages.collect();
    let description = [
        ("over_7d", "over seven days old"),
        ("1d_to_7d", "between one and seven days old"),
        ("1h_to_24h", "between one hour and one day old"),
        ("under_1h", "under an hour old"),
    ]
    .into_iter()
    .find_map(|(label, description)| ages.contains(label).then_some(description));
    let mut sentence = description
        .map(|description| format!("The oldest is {description}."))
        .unwrap_or_default();
    if ages.contains("future_timestamp") {
        if !sentence.is_empty() {
            sentence.push(' ');
        }
        sentence.push_str("Some billing row timestamps are in the future.");
    }
    sentence
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum InventoryIssue {
    RowLimit,
    Deadline,
    InvalidRows,
    InventoryRead,
    RegistryRead,
    RecoveryTotals,
    HoldlessCount,
}

impl InventoryIssue {
    pub(super) fn scan_failure(reason: &str) -> Self {
        match reason {
            "deadline_exceeded" | "query_timeout" => Self::Deadline,
            "row_decode_error" => Self::InvalidRows,
            _ => Self::InventoryRead,
        }
    }

    fn check(self) -> &'static str {
        match self {
            Self::RowLimit => {
                "the row limit in Details and the size of the unfinished-request backlog"
            }
            Self::Deadline => "query performance for the scan that reached its deadline",
            Self::InvalidRows => {
                "the stored billing rows for invalid fields or incompatible schemas"
            }
            Self::InventoryRead => {
                "database access and required indexes for the unreadable inventory"
            }
            Self::RegistryRead => {
                "the process registry for missing, invalid, or unreadable records"
            }
            Self::RecoveryTotals => "the stored recovery counters for invalid or unreadable values",
            Self::HoldlessCount => "the count query and required indexes for rows with no holds",
        }
    }
}

pub(super) fn inventory_status(
    issues: &std::collections::BTreeSet<InventoryIssue>,
) -> (super::DiagnosticSeverity, String) {
    if issues.is_empty() {
        return (
            super::DiagnosticSeverity::Info,
            "No action is needed. This inventory is read-only.".into(),
        );
    }
    let checks = issues
        .iter()
        .map(|issue| issue.check())
        .collect::<Vec<_>>()
        .join("; ");
    (
        super::DiagnosticSeverity::Action,
        format!(
            "Check {checks}. Check database health and server logs for the stages shown in Details, then reload this page to read the inventory again."
        ),
    )
}
