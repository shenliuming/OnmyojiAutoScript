CREATE TABLE login_session (
    id BIGINT NOT NULL AUTO_INCREMENT,
    session_no VARCHAR(64) NOT NULL,
    game_account_id BIGINT NOT NULL,
    binding_id BIGINT NOT NULL,
    emulator_id BIGINT NOT NULL,

    status VARCHAR(32) NOT NULL,
    public_token_hash CHAR(64) NOT NULL,
    control_token_hash CHAR(64) NOT NULL,

    qr_payload TEXT NULL,
    qr_expires_at DATETIME(3) NULL,

    detected_masked_account VARCHAR(255) NULL,
    detected_character_name VARCHAR(64) NULL,
    detected_server_name VARCHAR(64) NULL,
    detected_game_uid VARCHAR(64) NULL,

    expires_at DATETIME(3) NOT NULL,
    confirmed_at DATETIME(3) NULL,
    failed_reason VARCHAR(255) NULL,

    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),

    PRIMARY KEY (id),
    UNIQUE KEY uk_login_session_no (session_no),
    UNIQUE KEY uk_login_public_token_hash (public_token_hash),
    UNIQUE KEY uk_login_control_token_hash (control_token_hash),
    KEY idx_login_account_status (game_account_id, status),
    KEY idx_login_expiry (status, expires_at),

    CONSTRAINT fk_login_game_account
        FOREIGN KEY (game_account_id) REFERENCES game_account(id),
    CONSTRAINT fk_login_binding
        FOREIGN KEY (binding_id) REFERENCES emulator_account_binding(id),
    CONSTRAINT fk_login_emulator
        FOREIGN KEY (emulator_id) REFERENCES emulator_instance(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
