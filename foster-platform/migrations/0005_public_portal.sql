CREATE TABLE foster_share_link (
    id BIGINT NOT NULL AUTO_INCREMENT,
    subscription_id BIGINT NOT NULL,
    public_token_hash CHAR(64) NOT NULL,
    control_token_hash CHAR(64) NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'ACTIVE',
    expire_at DATETIME(3) NULL,
    last_access_at DATETIME(3) NULL,
    access_count BIGINT NOT NULL DEFAULT 0,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),

    PRIMARY KEY (id),
    UNIQUE KEY uk_share_public_token_hash (public_token_hash),
    UNIQUE KEY uk_share_control_token_hash (control_token_hash),
    KEY idx_share_subscription_status (subscription_id, status),
    CONSTRAINT fk_share_subscription
        FOREIGN KEY (subscription_id) REFERENCES foster_subscription(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
