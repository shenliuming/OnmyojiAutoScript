use std::collections::HashSet;

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

fn stored_values(stored: &[AccountIdentity], kind: IdentityType) -> HashSet<&str> {
    stored
        .iter()
        .filter(|identity| identity.kind == kind)
        .map(|identity| identity.normalized_value.as_str())
        .collect()
}

fn optional_match(stored: &[AccountIdentity], kind: IdentityType, detected: Option<&str>) -> bool {
    let Some(detected) = detected else {
        return false;
    };

    let expected = stored_values(stored, kind);
    if expected.is_empty() {
        return false;
    }

    let normalized = normalize_identity(kind, detected);
    expected.contains(normalized.as_str())
}

fn optional_conflict(
    stored: &[AccountIdentity],
    kind: IdentityType,
    detected: Option<&str>,
) -> bool {
    let Some(detected) = detected else {
        return false;
    };

    let expected = stored_values(stored, kind);
    if expected.is_empty() {
        return false;
    }

    let normalized = normalize_identity(kind, detected);
    !expected.contains(normalized.as_str())
}

fn account_signal_matches(stored: &[AccountIdentity], detected: &DetectedIdentity) -> bool {
    let expected: HashSet<&str> = stored
        .iter()
        .filter(|identity| {
            matches!(
                identity.kind,
                IdentityType::MaskedAccount | IdentityType::OcrAlias
            )
        })
        .map(|identity| identity.normalized_value.as_str())
        .collect();

    if expected.is_empty() {
        return false;
    }

    let mut candidates = Vec::new();
    if let Some(masked) = detected.masked_account.as_deref() {
        candidates.push(normalize_identity(IdentityType::MaskedAccount, masked));
    }
    candidates.extend(
        detected
            .ocr_aliases
            .iter()
            .map(|alias| normalize_identity(IdentityType::OcrAlias, alias)),
    );

    candidates
        .iter()
        .any(|candidate| expected.contains(candidate.as_str()))
}

pub fn verify_identity(
    stored: &[AccountIdentity],
    detected: &DetectedIdentity,
) -> IdentityDecision {
    let mut conflicting = Vec::new();

    if optional_conflict(stored, IdentityType::GameUid, detected.game_uid.as_deref()) {
        conflicting.push(IdentityType::GameUid);
    }
    if optional_conflict(
        stored,
        IdentityType::CharacterName,
        detected.character_name.as_deref(),
    ) {
        conflicting.push(IdentityType::CharacterName);
    }
    if optional_conflict(
        stored,
        IdentityType::ServerName,
        detected.server_name.as_deref(),
    ) {
        conflicting.push(IdentityType::ServerName);
    }

    if !conflicting.is_empty() {
        return IdentityDecision::Mismatch { conflicting };
    }

    let uid_match = optional_match(stored, IdentityType::GameUid, detected.game_uid.as_deref());
    let character_match = optional_match(
        stored,
        IdentityType::CharacterName,
        detected.character_name.as_deref(),
    );
    let server_match = optional_match(
        stored,
        IdentityType::ServerName,
        detected.server_name.as_deref(),
    );
    let account_match = account_signal_matches(stored, detected);

    let mut matched = Vec::new();
    if uid_match {
        matched.push(IdentityType::GameUid);
    }
    if character_match {
        matched.push(IdentityType::CharacterName);
    }
    if server_match {
        matched.push(IdentityType::ServerName);
    }
    if account_match {
        matched.push(IdentityType::MaskedAccount);
    }

    if uid_match
        || (character_match && server_match)
        || (account_match && (character_match || server_match))
    {
        IdentityDecision::Verified { matched }
    } else {
        IdentityDecision::Ambiguous
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
