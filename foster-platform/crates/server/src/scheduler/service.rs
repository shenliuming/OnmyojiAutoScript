use chrono::{DateTime, Utc};
use sqlx::MySqlPool;
use uuid::Uuid;

use super::repository::{
    clear_next_run, has_active_binding, has_nonterminal_job, insert_pending_job,
    list_due_subscription_ids, lock_due_subscription,
};

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct SchedulerService {
    pool: MySqlPool,
}

impl SchedulerService {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }

    pub async fn create_due_jobs(
        &self,
        now: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<i64>, SchedulerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let candidates = list_due_subscription_ids(&self.pool, now, limit).await?;
        let mut created = Vec::with_capacity(candidates.len());

        for subscription_id in candidates {
            let mut tx = self.pool.begin().await?;

            let Some(subscription) = lock_due_subscription(&mut tx, subscription_id, now).await?
            else {
                tx.rollback().await?;
                continue;
            };

            if !has_active_binding(&mut tx, subscription.game_account_id).await?
                || has_nonterminal_job(&mut tx, subscription.id).await?
            {
                tx.rollback().await?;
                continue;
            }

            let job_no = format!("JOB-{}", Uuid::new_v4());
            let job_id = insert_pending_job(
                &mut tx,
                &job_no,
                subscription.id,
                subscription.game_account_id,
                subscription.next_run_at,
            )
            .await?;

            clear_next_run(&mut tx, subscription.id).await?;
            tx.commit().await?;
            created.push(job_id);
        }

        Ok(created)
    }
}
