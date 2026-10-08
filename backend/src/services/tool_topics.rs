use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{DownstreamService, OfferingKind};

pub const TOOL_TOPICS: &[(&str, &str)] = &[
    ("web-search", "Web Search"),
    ("page-fetch", "Page Fetch"),
    ("news", "News"),
    ("company-data", "Company Data"),
    ("people-data", "People Data"),
    ("contact-enrichment", "Contact Enrichment"),
    ("seo", "Seo"),
    ("social", "Social"),
    ("jobs", "Jobs"),
    ("real-estate", "Real Estate"),
    ("automotive", "Automotive"),
    ("finance", "Finance"),
    ("crypto", "Crypto"),
    ("weather", "Weather"),
    ("air-quality", "Air Quality"),
    ("reviews", "Reviews"),
    ("software-directory", "Software Directory"),
    ("research-papers", "Research Papers"),
    ("government-data", "Government Data"),
    ("economic-data", "Economic Data"),
    ("llm-benchmarks", "Llm Benchmarks"),
    ("generation-image", "Generation Image"),
    ("generation-video", "Generation Video"),
    ("generation-audio", "Generation Audio"),
    ("email", "Email"),
    ("phone", "Phone"),
    ("browser", "Browser"),
    ("machine", "Machine"),
    ("storage", "Storage"),
];

pub fn validate_tool_service(service: &DownstreamService) -> AppResult<()> {
    if service.topics.len() > 20 {
        return Err(AppError::ValidationError(
            "topics allows at most 20 entries".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for topic in &service.topics {
        if !TOOL_TOPICS.iter().any(|(slug, _)| slug == topic) || !seen.insert(topic) {
            return Err(AppError::ValidationError(format!(
                "Invalid or duplicate topic: {topic}"
            )));
        }
    }
    if service
        .supplier
        .as_ref()
        .is_some_and(|s| s.chars().count() > 128)
    {
        return Err(AppError::ValidationError(
            "supplier exceeds 128 characters".into(),
        ));
    }
    if let Some(source) = &service.import_source {
        if source.reference.chars().count() > 512
            || source
                .version
                .as_ref()
                .is_some_and(|s| s.chars().count() > 128)
        {
            return Err(AppError::ValidationError(
                "import_source exceeds length limits".into(),
            ));
        }
    }
    if service.offering_kind == OfferingKind::Tool
        && (service.service_category != "internal"
            || (service.auth_method != "none"
                && service.credential_encrypted.is_empty()
                && service.platform_key.is_none()))
    {
        return Err(AppError::ValidationError("tool offerings require internal service_category and no auth, a master credential, or platform_key configuration".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabulary_is_unique_and_lowercase_kebab() {
        let mut seen = std::collections::HashSet::new();
        for (slug, label) in TOOL_TOPICS {
            assert!((2..=40).contains(&slug.len()));
            assert!(slug.bytes().all(|c| c.is_ascii_lowercase() || c == b'-'));
            assert!(seen.insert(slug));
            assert!(!label.is_empty());
        }
    }

    #[test]
    fn tools_require_internal_execution_and_configured_auth_boundary() {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.offering_kind = OfferingKind::Tool;
        assert!(validate_tool_service(&service).is_err());
        service.service_category = "internal".into();
        service.auth_method = "none".into();
        assert!(validate_tool_service(&service).is_ok());
        service.auth_method = "bearer".into();
        service.credential_encrypted = Vec::new();
        service.platform_key = None;
        assert!(validate_tool_service(&service).is_err());
        service.platform_key = Some(Default::default());
        assert!(validate_tool_service(&service).is_ok());
        service.topics = vec!["social".into(), "social".into()];
        assert!(validate_tool_service(&service).is_err());
        service.topics = vec!["unknown".into()];
        assert!(validate_tool_service(&service).is_err());
    }
}
