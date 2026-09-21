CREATE TABLE host (
    id BIGINT NOT NULL AUTO_INCREMENT,
    host_code VARCHAR(64) NOT NULL,
    hostname VARCHAR(128) NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'OFFLINE',
    agent_version VARCHAR(64) NULL,
    last_heartbeat_at DATETIME(3) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),
    PRIMARY KEY (id),
    UNIQUE KEY uk_host_code (host_code)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE emulator_instance (
    id BIGINT NOT NULL AUTO_INCREMENT,
    host_id BIGINT NOT NULL,
    emulator_code VARCHAR(64) NOT NULL,
    driver_type VARCHAR(32) NOT NULL DEFAULT 'UNKNOWN',
    max_account_count INT NOT NULL DEFAULT 5,
    status VARCHAR(32) NOT NULL DEFAULT 'OFFLINE',
    adb_serial VARCHAR(128) NULL,
    current_job_id VARCHAR(64) NULL,
    last_heartbeat_at DATETIME(3) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),
    PRIMARY KEY (id),
    UNIQUE KEY uk_emulator_code (emulator_code),
    KEY idx_emulator_host_status (host_id, status),
    CONSTRAINT fk_emulator_host
        FOREIGN KEY (host_id) REFERENCES host(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE game_account (
    id BIGINT NOT NULL AUTO_INCREMENT,
    customer_id BIGINT NOT NULL,
    character_name VARCHAR(64) NULL,
    server_name VARCHAR(64) NULL,
    game_uid VARCHAR(64) NULL,
    platform VARCHAR(32) NULL,
    login_status VARCHAR(32) NOT NULL DEFAULT 'PENDING',
    verify_status VARCHAR(32) NOT NULL DEFAULT 'PENDING',
    active_emulator_id BIGINT NULL,
    last_verified_at DATETIME(3) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),
    PRIMARY KEY (id),
    KEY idx_game_account_customer (customer_id),
    KEY idx_game_account_active_emulator (active_emulator_id),
    CONSTRAINT fk_game_account_active_emulator
        FOREIGN KEY (active_emulator_id) REFERENCES emulator_instance(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE game_account_identity (
    id BIGINT NOT NULL AUTO_INCREMENT,
    game_account_id BIGINT NOT NULL,
    identity_type VARCHAR(32) NOT NULL,
    identity_value VARCHAR(255) NOT NULL,
    normalized_value VARCHAR(255) NOT NULL,
    source VARCHAR(32) NOT NULL,
    confidence INT NOT NULL DEFAULT 100,
    enabled TINYINT(1) NOT NULL DEFAULT 1,
    last_seen_at DATETIME(3) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    PRIMARY KEY (id),
    KEY idx_identity_account (game_account_id),
    KEY idx_identity_lookup (identity_type, normalized_value, enabled),
    CONSTRAINT fk_identity_account
        FOREIGN KEY (game_account_id) REFERENCES game_account(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE emulator_account_binding (
    id BIGINT NOT NULL AUTO_INCREMENT,
    emulator_id BIGINT NOT NULL,
    game_account_id BIGINT NOT NULL,
    slot_no INT NOT NULL,
    status VARCHAR(32) NOT NULL,
    bound_at DATETIME(3) NULL,
    unbound_at DATETIME(3) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),

    active_account_guard BIGINT
        GENERATED ALWAYS AS (
            CASE
                WHEN status = 'ACTIVE' THEN game_account_id
                ELSE NULL
            END
        ) STORED,

    occupied_slot_guard VARCHAR(128)
        GENERATED ALWAYS AS (
            CASE
                WHEN status IN ('PENDING', 'ACTIVE', 'MIGRATING')
                THEN CONCAT(emulator_id, ':', slot_no)
                ELSE NULL
            END
        ) STORED,

    PRIMARY KEY (id),
    UNIQUE KEY uk_one_active_binding_per_account (active_account_guard),
    UNIQUE KEY uk_one_occupied_binding_per_slot (occupied_slot_guard),
    KEY idx_binding_account_status (game_account_id, status),
    KEY idx_binding_emulator_status (emulator_id, status),
    CONSTRAINT fk_binding_emulator
        FOREIGN KEY (emulator_id) REFERENCES emulator_instance(id),
    CONSTRAINT fk_binding_account
        FOREIGN KEY (game_account_id) REFERENCES game_account(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
