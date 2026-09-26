ALTER TABLE emulator_instance
    ADD COLUMN lifecycle_status VARCHAR(32) NOT NULL DEFAULT 'OFFLINE' AFTER status,
    ADD COLUMN occupancy_status VARCHAR(32) NOT NULL DEFAULT 'IDLE' AFTER lifecycle_status,
    ADD COLUMN activity_type VARCHAR(32) NOT NULL DEFAULT 'NONE' AFTER occupancy_status,
    ADD COLUMN activity_stage VARCHAR(64) NULL AFTER activity_type,
    ADD COLUMN current_command_id CHAR(36) NULL AFTER activity_stage,
    ADD COLUMN current_foster_job_id BIGINT NULL AFTER current_command_id,
    ADD COLUMN current_login_session_no VARCHAR(128) NULL AFTER current_foster_job_id,
    ADD COLUMN current_game_account_id BIGINT NULL AFTER current_login_session_no,
    ADD COLUMN activity_started_at DATETIME(3) NULL AFTER current_game_account_id;

UPDATE emulator_instance
SET lifecycle_status = CASE
        WHEN status = 'OFFLINE' THEN 'OFFLINE'
        WHEN status = 'MAINTENANCE' THEN 'MAINTENANCE'
        WHEN status = 'ERROR' THEN 'ERROR'
        ELSE 'READY'
    END,
    occupancy_status = CASE
        WHEN status IN ('RUNNING', 'SWITCHING_ACCOUNT', 'LOGIN_SESSION') THEN 'BUSY'
        WHEN status = 'MAINTENANCE' THEN 'MAINTENANCE'
        ELSE 'IDLE'
    END,
    activity_type = CASE
        WHEN status = 'LOGIN_SESSION' THEN 'LOGIN'
        WHEN status IN ('RUNNING', 'SWITCHING_ACCOUNT') THEN 'FOSTER'
        ELSE 'NONE'
    END;

CREATE TABLE emulator_lease (
    emulator_id BIGINT NOT NULL,
    lease_token CHAR(36) NOT NULL,
    owner_type VARCHAR(32) NOT NULL,
    owner_key VARCHAR(128) NOT NULL,
    command_id CHAR(36) NOT NULL,
    acquired_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    expires_at DATETIME(3) NOT NULL,
    PRIMARY KEY (emulator_id),
    UNIQUE KEY uk_emulator_lease_token (lease_token),
    KEY idx_emulator_lease_owner (owner_type, owner_key),
    KEY idx_emulator_lease_expiry (expires_at),
    CONSTRAINT fk_emulator_lease_emulator
        FOREIGN KEY (emulator_id) REFERENCES emulator_instance(id)
        ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
