use crate::{
    errors::{AppError, AppResult},
    models::downstream_service::{CatalogImportKind, CatalogImportSource},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CatalogImportSourceDto {
    pub kind: CatalogImportKind,
    pub reference: String,
    pub version: Option<String>,
    pub imported_at: Option<String>,
}

impl CatalogImportSourceDto {
    pub fn into_model(self) -> AppResult<CatalogImportSource> {
        let imported_at = self
            .imported_at
            .as_deref()
            .map(|value| {
                chrono::DateTime::parse_from_rfc3339(value)
                    .map(|date| date.with_timezone(&chrono::Utc))
                    .map_err(|_| {
                        AppError::ValidationError(
                            "import_source.imported_at must be RFC3339".into(),
                        )
                    })
            })
            .transpose()?;
        Ok(CatalogImportSource {
            kind: self.kind,
            reference: self.reference,
            version: self.version,
            imported_at,
        })
    }
}

impl From<CatalogImportSource> for CatalogImportSourceDto {
    fn from(source: CatalogImportSource) -> Self {
        Self {
            kind: source.kind,
            reference: source.reference,
            version: source.version,
            imported_at: source.imported_at.map(|date| date.to_rfc3339()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn import_dates_are_json_strings_and_bson_datetimes() {
        let input: CatalogImportSourceDto = serde_json::from_value(serde_json::json!({
            "kind":"manual", "reference":"date-proof", "imported_at":"2026-10-09T08:00:00+08:00"
        }))
        .unwrap();
        let model = input.into_model().unwrap();
        let stored = bson::to_document(&model).unwrap();
        assert_eq!(
            stored
                .get_datetime("imported_at")
                .unwrap()
                .timestamp_millis(),
            model.imported_at.as_ref().unwrap().timestamp_millis()
        );
        let response = serde_json::to_value(CatalogImportSourceDto::from(model)).unwrap();
        assert_eq!(response["imported_at"], "2026-10-09T00:00:00+00:00");
    }
    #[test]
    fn import_dates_validate_and_allow_omission() {
        let mut input: CatalogImportSourceDto = serde_json::from_value(serde_json::json!({
            "kind":"manual", "reference":"date-proof"
        }))
        .unwrap();
        assert!(input.clone().into_model().unwrap().imported_at.is_none());
        input.imported_at = Some("not-a-date".into());
        assert!(matches!(
            input.into_model(),
            Err(AppError::ValidationError(_))
        ));
    }
}
