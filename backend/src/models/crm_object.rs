use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "crm_objects";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CrmObject {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub key: String,
    pub label: String,
    pub plural_label: String,
    pub schema_revision: i64,
    pub fields: Vec<CrmField>,
    pub pipeline: Option<CrmPipeline>,
    pub default_columns: Vec<String>,
    pub seeded: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CrmField {
    pub key: String,
    pub label: String,
    pub required: bool,
    pub archived: bool,
    #[serde(flatten)]
    pub kind: CrmFieldKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CrmFieldKind {
    Text {
        #[serde(default = "default_text_max_length")]
        max_length: u32,
    },
    Number {
        format: CrmNumberFormat,
    },
    Currency {
        currency: String,
    },
    Boolean {},
    Date {},
    Datetime {},
    Select {
        options: Vec<CrmSelectOption>,
    },
    Record {
        target: String,
        cardinality: CrmCardinality,
    },
    User {},
}

fn default_text_max_length() -> u32 {
    1000
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CrmNumberFormat {
    Plain,
    Percent,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CrmCardinality {
    One,
    Many,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrmSelectOption {
    pub value: String,
    pub label: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrmPipeline {
    pub stage_field: String,
    pub closed_stages: Vec<String>,
}
