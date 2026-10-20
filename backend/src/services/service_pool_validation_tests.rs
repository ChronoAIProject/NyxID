use super::*;

#[test]
fn pool_text_limits_count_unicode_characters() {
    for character in ["a", "服", "🪐"] {
        assert!(
            validate_text_fields(&character.repeat(128), Some(&character.repeat(1024))).is_ok()
        );
        assert!(validate_text_fields(&character.repeat(129), None).is_err());
        assert!(validate_text_fields("Pool", Some(&character.repeat(1025))).is_err());
        let member = ServicePoolMember {
            user_service_id: "member".into(),
            weight: 1,
            enabled: true,
            priority: 0,
            model: Some(format!("  {}  ", character.repeat(256))),
            same_api_compatible: false,
            health_reset_generation: 0,
        };
        let normalized = normalize_members(vec![member.clone()]).unwrap();
        assert_eq!(
            normalized[0].model.as_deref(),
            Some(character.repeat(256).as_str())
        );
        assert!(
            normalize_members(vec![ServicePoolMember {
                model: Some(character.repeat(257)),
                ..member
            }])
            .is_err()
        );
    }
    assert!(validate_text_fields("  ", None).is_err());
}
