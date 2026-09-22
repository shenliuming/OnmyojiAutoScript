CREATE TABLE provider_account (
    id BIGINT NOT NULL AUTO_INCREMENT,
    provider_code VARCHAR(64) NOT NULL,
    game_uid VARCHAR(64) NULL,
    nickname VARCHAR(64) NOT NULL,
    provider_alias VARCHAR(64) NOT NULL,
    server_name VARCHAR(64) NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'ACTIVE',
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),

    PRIMARY KEY (id),
    UNIQUE KEY uk_provider_code (provider_code),
    UNIQUE KEY uk_provider_alias (provider_alias),
    KEY idx_provider_status (status)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE foster_friend_binding (
    id BIGINT NOT NULL AUTO_INCREMENT,
    game_account_id BIGINT NOT NULL,
    provider_account_id BIGINT NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'VERIFIED',
    verified_at DATETIME(3) NULL,
    last_failure_at DATETIME(3) NULL,
    last_failure_code VARCHAR(64) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),

    PRIMARY KEY (id),
    UNIQUE KEY uk_friend_binding_account_provider (game_account_id, provider_account_id),
    KEY idx_friend_binding_account_status (game_account_id, status),
    KEY idx_friend_binding_provider_status (provider_account_id, status),
    CONSTRAINT fk_friend_binding_account
        FOREIGN KEY (game_account_id) REFERENCES game_account(id),
    CONSTRAINT fk_friend_binding_provider
        FOREIGN KEY (provider_account_id) REFERENCES provider_account(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE foster_resource_cycle (
    id BIGINT NOT NULL AUTO_INCREMENT,
    provider_account_id BIGINT NOT NULL,
    resource_type VARCHAR(32) NOT NULL,
    resource_level INT NOT NULL DEFAULT 0,
    start_at DATETIME(3) NOT NULL,
    end_at DATETIME(3) NOT NULL,
    slot_capacity INT NOT NULL,
    occupied_slots INT NOT NULL DEFAULT 0,
    status VARCHAR(32) NOT NULL DEFAULT 'AVAILABLE',
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),

    PRIMARY KEY (id),
    KEY idx_resource_cycle_type_status_end (resource_type, status, end_at),
    KEY idx_resource_cycle_provider_status (provider_account_id, status),
    CONSTRAINT fk_resource_cycle_provider
        FOREIGN KEY (provider_account_id) REFERENCES provider_account(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE foster_resource_allocation (
    id BIGINT NOT NULL AUTO_INCREMENT,
    job_id BIGINT NOT NULL,
    resource_cycle_id BIGINT NOT NULL,
    provider_account_id BIGINT NOT NULL,
    status VARCHAR(32) NOT NULL,
    reserved_at DATETIME(3) NOT NULL,
    confirmed_at DATETIME(3) NULL,
    released_at DATETIME(3) NULL,
    occupied_until DATETIME(3) NULL,
    release_reason VARCHAR(255) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),

    active_job_guard BIGINT
        GENERATED ALWAYS AS (
            CASE
                WHEN status IN ('RESERVED', 'CONFIRMED') THEN job_id
                ELSE NULL
            END
        ) STORED,

    PRIMARY KEY (id),
    UNIQUE KEY uk_allocation_one_live_per_job (active_job_guard),
    KEY idx_allocation_job_status (job_id, status),
    KEY idx_allocation_cycle_status (resource_cycle_id, status),
    KEY idx_allocation_occupied_until (status, occupied_until),
    CONSTRAINT fk_allocation_job
        FOREIGN KEY (job_id) REFERENCES foster_job(id),
    CONSTRAINT fk_allocation_cycle
        FOREIGN KEY (resource_cycle_id) REFERENCES foster_resource_cycle(id),
    CONSTRAINT fk_allocation_provider
        FOREIGN KEY (provider_account_id) REFERENCES provider_account(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

ALTER TABLE foster_job
    ADD COLUMN provider_account_id BIGINT NULL AFTER emulator_id,
    ADD COLUMN resource_cycle_id BIGINT NULL AFTER provider_account_id,
    ADD COLUMN resource_allocation_id BIGINT NULL AFTER resource_cycle_id,
    ADD KEY idx_job_provider (provider_account_id),
    ADD KEY idx_job_resource_cycle (resource_cycle_id),
    ADD KEY idx_job_resource_allocation (resource_allocation_id),
    ADD CONSTRAINT fk_job_provider
        FOREIGN KEY (provider_account_id) REFERENCES provider_account(id),
    ADD CONSTRAINT fk_job_resource_cycle
        FOREIGN KEY (resource_cycle_id) REFERENCES foster_resource_cycle(id);
