CREATE TABLE foster_plan (
    id BIGINT NOT NULL AUTO_INCREMENT,
    plan_code VARCHAR(64) NOT NULL,
    plan_name VARCHAR(128) NOT NULL,
    daily_target_runs INT NOT NULL DEFAULT 4,
    interval_minutes INT NOT NULL DEFAULT 360,
    resource_mode VARCHAR(32) NOT NULL,
    resource_type VARCHAR(32) NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'ACTIVE',
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),

    PRIMARY KEY (id),
    UNIQUE KEY uk_foster_plan_code (plan_code)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE foster_subscription (
    id BIGINT NOT NULL AUTO_INCREMENT,
    subscription_no VARCHAR(64) NOT NULL,
    game_account_id BIGINT NOT NULL,
    plan_id BIGINT NOT NULL,

    resource_mode VARCHAR(32) NOT NULL,
    resource_type VARCHAR(32) NULL,
    daily_target_runs INT NOT NULL DEFAULT 4,
    interval_minutes INT NOT NULL DEFAULT 360,

    status VARCHAR(32) NOT NULL,
    last_success_at DATETIME(3) NULL,
    next_run_at DATETIME(3) NULL,
    manual_pause_until DATETIME(3) NULL,
    start_at DATETIME(3) NOT NULL,
    end_at DATETIME(3) NOT NULL,

    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),

    PRIMARY KEY (id),
    UNIQUE KEY uk_foster_subscription_no (subscription_no),
    KEY idx_subscription_due (status, next_run_at),
    KEY idx_subscription_account (game_account_id),
    CONSTRAINT fk_subscription_account
        FOREIGN KEY (game_account_id) REFERENCES game_account(id),
    CONSTRAINT fk_subscription_plan
        FOREIGN KEY (plan_id) REFERENCES foster_plan(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE foster_quiet_period (
    id BIGINT NOT NULL AUTO_INCREMENT,
    game_account_id BIGINT NOT NULL,
    weekday_mask TINYINT UNSIGNED NOT NULL DEFAULT 127,
    start_time TIME NOT NULL,
    end_time TIME NOT NULL,
    timezone VARCHAR(64) NOT NULL DEFAULT 'Asia/Shanghai',
    before_buffer_minutes INT NOT NULL DEFAULT 0,
    after_buffer_minutes INT NOT NULL DEFAULT 0,
    enabled TINYINT(1) NOT NULL DEFAULT 1,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),

    PRIMARY KEY (id),
    KEY idx_quiet_account_enabled (game_account_id, enabled),
    CONSTRAINT fk_quiet_account
        FOREIGN KEY (game_account_id) REFERENCES game_account(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE foster_job (
    id BIGINT NOT NULL AUTO_INCREMENT,
    job_no VARCHAR(64) NOT NULL,
    subscription_id BIGINT NOT NULL,
    game_account_id BIGINT NOT NULL,
    emulator_id BIGINT NULL,

    status VARCHAR(32) NOT NULL,
    scheduled_at DATETIME(3) NOT NULL,
    deferred_until DATETIME(3) NULL,
    started_at DATETIME(3) NULL,
    finished_at DATETIME(3) NULL,

    retry_count INT NOT NULL DEFAULT 0,
    retry_after DATETIME(3) NULL,
    error_code VARCHAR(64) NULL,
    result_message VARCHAR(512) NULL,
    remaining_seconds INT NULL,
    screenshot_url VARCHAR(1024) NULL,

    nonterminal_subscription_guard BIGINT
        GENERATED ALWAYS AS (
            CASE
                WHEN status IN (
                    'PENDING',
                    'DEFERRED_QUIET',
                    'DEFERRED_MANUAL',
                    'WAITING_EMULATOR',
                    'WAITING_RESOURCE',
                    'SWITCHING_ACCOUNT',
                    'VERIFYING_ACCOUNT',
                    'RUNNING',
                    'RETRY',
                    'RECOVERY_REQUIRED'
                )
                THEN subscription_id
                ELSE NULL
            END
        ) STORED,

    executing_account_guard BIGINT
        GENERATED ALWAYS AS (
            CASE
                WHEN status IN (
                    'SWITCHING_ACCOUNT',
                    'VERIFYING_ACCOUNT',
                    'RUNNING'
                )
                THEN game_account_id
                ELSE NULL
            END
        ) STORED,

    executing_emulator_guard BIGINT
        GENERATED ALWAYS AS (
            CASE
                WHEN emulator_id IS NOT NULL
                 AND status IN (
                    'SWITCHING_ACCOUNT',
                    'VERIFYING_ACCOUNT',
                    'RUNNING'
                 )
                THEN emulator_id
                ELSE NULL
            END
        ) STORED,

    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),

    PRIMARY KEY (id),
    UNIQUE KEY uk_foster_job_no (job_no),
    UNIQUE KEY uk_job_one_nonterminal_subscription (nonterminal_subscription_guard),
    UNIQUE KEY uk_job_one_executing_account (executing_account_guard),
    UNIQUE KEY uk_job_one_executing_emulator (executing_emulator_guard),
    KEY idx_job_status_schedule (status, scheduled_at),
    KEY idx_job_subscription (subscription_id),
    KEY idx_job_account (game_account_id),
    KEY idx_job_emulator (emulator_id),

    CONSTRAINT fk_job_subscription
        FOREIGN KEY (subscription_id) REFERENCES foster_subscription(id),
    CONSTRAINT fk_job_account
        FOREIGN KEY (game_account_id) REFERENCES game_account(id),
    CONSTRAINT fk_job_emulator
        FOREIGN KEY (emulator_id) REFERENCES emulator_instance(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
