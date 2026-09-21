use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IdentityType {
    MaskedAccount,
    OcrAlias,
    CharacterName,
    ServerName,
    GameUid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    pub kind: IdentityType,
    pub value: String,
    pub normalized_value: String,
    pub confidence: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DetectedIdentity {
    pub masked_account: Option<String>,
    pub character_name: Option<String>,
    pub server_name: Option<String>,
    pub game_uid: Option<String>,
    pub ocr_aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityDecision {
    Verified { matched: Vec<IdentityType> },
    Ambiguous,
    Mismatch { conflicting: Vec<IdentityType> },
}

pub fn normalize_identity(kind: IdentityType, value: &str) -> String {
    match kind {
        IdentityType::MaskedAccount | IdentityType::OcrAlias => {
            value.trim().to_ascii_lowercase().replace(' ', "")
        }
        _ => value.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(kind: IdentityType, value: &str) -> AccountIdentity {
        AccountIdentity {
            kind,
            value: value.to_string(),
            normalized_value: normalize_identity(kind, value),
            confidence: 100,
        }
    }

    #[test]
    fn masked_account_alone_is_ambiguous() {
        let stored = vec![identity(IdentityType::MaskedAccount, "138****5678")];
        let detected = DetectedIdentity {
            masked_account: Some("138****5678".into()),
            ..Default::default()
        };

        assert_eq!(
            verify_identity(&stored, &detected),
            IdentityDecision::Ambiguous
        );
    }

    #[test]
    fn masked_account_plus_character_verifies() {
        let stored = vec![
            identity(IdentityType::MaskedAccount, "138****5678"),
            identity(IdentityType::CharacterName, "柳某某"),
        ];
        let detected = DetectedIdentity {
            masked_account: Some("138****5678".into()),
            character_name: Some("柳某某".into()),
            ..Default::default()
        };

        assert!(matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Verified { .. }
        ));
    }

    #[test]
    fn character_server_pair_verifies_without_masked_account() {
        let stored = vec![
            identity(IdentityType::CharacterName, "柳某某"),
            identity(IdentityType::ServerName, "春之樱"),
        ];
        let detected = DetectedIdentity {
            character_name: Some("柳某某".into()),
            server_name: Some("春之樱".into()),
            ..Default::default()
        };

        assert!(matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Verified { .. }
        ));
    }

    #[test]
    fn uid_match_verifies() {
        let stored = vec![identity(IdentityType::GameUid, "10001")];
        let detected = DetectedIdentity {
            game_uid: Some("10001".into()),
            ..Default::default()
        };

        assert!(matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Verified { .. }
        ));
    }

    #[test]
    fn uid_conflict_is_mismatch_even_when_mask_matches() {
        let stored = vec![
            identity(IdentityType::MaskedAccount, "138****5678"),
            identity(IdentityType::GameUid, "10001"),
        ];
        let detected = DetectedIdentity {
            masked_account: Some("138****5678".into()),
            game_uid: Some("99999".into()),
            ..Default::default()
        };

        assert!(matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Mismatch { .. }
        ));
    }

    #[test]
    fn character_conflict_is_mismatch() {
        let stored = vec![
            identity(IdentityType::MaskedAccount, "138****5678"),
            identity(IdentityType::CharacterName, "角色A"),
        ];
        let detected = DetectedIdentity {
            masked_account: Some("138****5678".into()),
            character_name: Some("角色B".into()),
            ..Default::default()
        };

        assert!(matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Mismatch { .. }
        ));
    }

    #[test]
    fn ocr_alias_plus_server_verifies() {
        let stored = vec![
            identity(IdentityType::OcrAlias, "138****S678"),
            identity(IdentityType::ServerName, "春之樱"),
        ];
        let detected = DetectedIdentity {
            server_name: Some("春之樱".into()),
            ocr_aliases: vec!["138****S678".into()],
            ..Default::default()
        };

        assert!(matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Verified { .. }
        ));
    }
}
