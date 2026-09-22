INSERT INTO foster_plan(
    plan_code, plan_name, daily_target_runs,
    interval_minutes, resource_mode, resource_type, status
)
VALUES
    (
        'BASIC_AUTO_FOSTER',
        '自动寄养',
        4,
        360,
        'USER_FRIEND',
        NULL,
        'ACTIVE'
    ),
    (
        'PLATFORM_FISH',
        '斗鱼资源寄养',
        4,
        360,
        'PLATFORM',
        'FISH',
        'ACTIVE'
    ),
    (
        'PLATFORM_TAIKO_JADE',
        '勾玉资源寄养',
        4,
        360,
        'PLATFORM',
        'TAIKO_JADE',
        'ACTIVE'
    );
