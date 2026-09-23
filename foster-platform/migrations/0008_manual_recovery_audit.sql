CREATE TABLE foster_recovery_audit (
    id BIGINT NOT NULL AUTO_INCREMENT,
    job_id BIGINT NOT NULL,
    expected_attempt INT NOT NULL,
    resolution VARCHAR(32) NOT NULL,
    operator_name VARCHAR(64) NOT NULL,
    operator_note VARCHAR(255) NOT NULL,
    observed_remaining_seconds INT NULL,
    prior_allocation_id BIGINT NULL,
    resolved_at DATETIME(3) NOT NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),

    PRIMARY KEY (id),
    UNIQUE KEY uk_recovery_job_attempt (job_id, expected_attempt),
    KEY idx_recovery_resolved_at (resolved_at),
    CONSTRAINT fk_recovery_job
        FOREIGN KEY (job_id) REFERENCES foster_job(id),
    CONSTRAINT fk_recovery_allocation
        FOREIGN KEY (prior_allocation_id) REFERENCES foster_resource_allocation(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
