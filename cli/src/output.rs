use crate::cli::OutputFormat;
use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

pub fn print_rows(
    value: &Value,
    format: OutputFormat,
    collection: Option<&str>,
    columns: &[(&str, &str)],
) -> Result<()> {
    if matches!(format, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(value)?);
        return Ok(());
    }
    let rows = collection.map(|key| &value[key]).unwrap_or(value);
    let singleton = vec![rows.clone()];
    let rows = rows.as_array().unwrap_or(&singleton);
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(columns.iter().map(|(label, _)| *label));
    for row in rows {
        table.add_row(
            columns
                .iter()
                .map(|(_, key)| match &row[*key] {
                    Value::String(s) => s.clone(),
                    Value::Null => "-".into(),
                    v => v.to_string(),
                })
                .collect::<Vec<_>>(),
        );
    }
    println!("{table}");
    Ok(())
}
