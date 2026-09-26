ALTER TABLE login_session
    ADD COLUMN expected_server_name VARCHAR(64) NULL AFTER detected_game_uid,
    ADD COLUMN expected_character_name VARCHAR(64) NULL AFTER expected_server_name,
    ADD COLUMN expected_game_uid VARCHAR(64) NULL AFTER expected_character_name,
    ADD COLUMN identity_verified TINYINT(1) NOT NULL DEFAULT 0 AFTER expected_game_uid,
    ADD COLUMN identity_verify_reason VARCHAR(255) NULL AFTER identity_verified;
