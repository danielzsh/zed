use crate::{db::UserId, Executor, Result};
use crate::{Database, Error};
use anyhow::anyhow;
use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use parking_lot::Mutex;
use sea_orm::prelude::DateTimeUtc;
use std::any::TypeId;
use std::sync::Arc;
use util::ResultExt;

pub trait RateLimit: 'static {
    fn capacity() -> usize;
    fn refill_duration() -> Duration;
    fn db_name() -> &'static str;
    fn type_id() -> TypeId {
        TypeId::of::<Self>()
    }
}

/// Used to enforce per-user rate limits
pub struct RateLimiter {
    buckets: DashMap<(UserId, TypeId), Arc<Mutex<RateBucket>>>,
    db: Arc<Database>,
    executor: Executor,
}

impl RateLimiter {
    pub fn new(db: Arc<Database>, executor: Executor) -> Self {
        RateLimiter {
            buckets: DashMap::new(),
            db,
            executor,
        }
    }

    /// Returns an error if the user has exceeded the specified `RateLimit`.
    /// Attempts to read the from the database if no cached RateBucket currently exists.
    pub async fn check<T: RateLimit>(&self, user_id: UserId) -> Result<()> {
        self.check_internal::<T>(user_id, Utc::now()).await
    }

    async fn check_internal<T: RateLimit>(&self, user_id: UserId, now: DateTimeUtc) -> Result<()> {
        let type_id = T::type_id();
        let bucket_key = (user_id, type_id);

        // Attempt to fetch the bucket from the database if it hasn't been cached.
        // For now, we keep buckets in memory for the lifetime of the process rather than expiring them,
        // but this enforces limits across restarts so long as the database is reachable.
        if !self.buckets.contains_key(&bucket_key) {
            if let Some(bucket) = self.load_bucket::<T>(user_id).await.log_err().flatten() {
                self.buckets
                    .insert(bucket_key, Arc::new(Mutex::new(bucket)));
            }
        }

        let bucket = self
            .buckets
            .entry(bucket_key)
            .or_insert_with(|| {
                Arc::new(Mutex::new(RateBucket::new(
                    T::capacity(),
                    T::refill_duration(),
                    now,
                )))
            })
            .value()
            .clone();

        let mut lock = bucket.lock();
        let allowed = lock.allow(now);
        let token_count = lock.token_count;
        let last_refill = lock.last_refill.naive_utc();
        drop(lock);

        // Perform a non-blocking save of the rate bucket to the database in its new state.
        let db = self.db.clone();
        self.executor.spawn_detached(async move {
            db.save_rate_bucket(user_id, T::db_name(), token_count as i32, last_refill)
                .await
                .log_err();
        });

        if !allowed {
            Err(anyhow!("rate limit exceeded"))?
        }

        Ok(())
    }

    async fn load_bucket<K: RateLimit>(
        &self,
        user_id: UserId,
    ) -> Result<Option<RateBucket>, Error> {
        Ok(self
            .db
            .get_rate_bucket(user_id, K::db_name())
            .await?
            .map(|saved_bucket| RateBucket {
                capacity: K::capacity(),
                refill_time_per_token: K::refill_duration(),
                token_count: saved_bucket.token_count as usize,
                last_refill: DateTime::from_naive_utc_and_offset(saved_bucket.last_refill, Utc),
            }))
    }
}

struct RateBucket {
    capacity: usize,
    token_count: usize,
    refill_time_per_token: Duration,
    last_refill: DateTimeUtc,
}

impl RateBucket {
    fn new(capacity: usize, refill_duration: Duration, now: DateTimeUtc) -> Self {
        RateBucket {
            capacity,
            token_count: capacity,
            refill_time_per_token: refill_duration / capacity as i32,
            last_refill: now,
        }
    }

    fn allow(&mut self, now: DateTimeUtc) -> bool {
        self.refill(now);
        if self.token_count > 0 {
            self.token_count -= 1;
            true
        } else {
            false
        }
    }

    fn refill(&mut self, now: DateTimeUtc) {
        let elapsed = now - self.last_refill;
        if elapsed >= self.refill_time_per_token {
            let new_tokens =
                elapsed.num_milliseconds() / self.refill_time_per_token.num_milliseconds();

            self.token_count = (self.token_count + new_tokens as usize).min(self.capacity);
            self.last_refill = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{NewUserParams, TestDb};
    use gpui::TestAppContext;

    #[gpui::test]
    async fn test_rate_limiter(cx: &mut TestAppContext) {
        let executor = Executor::Deterministic(cx.executor());
        let test_db = TestDb::sqlite(cx.executor().clone());
        let db = test_db.db().clone();
        let user_1 = db
            .create_user(
                "user-1@zed.dev",
                false,
                NewUserParams {
                    github_login: "user-1".into(),
                    github_user_id: 1,
                },
            )
            .await
            .unwrap()
            .user_id;
        let user_2 = db
            .create_user(
                "user-2@zed.dev",
                false,
                NewUserParams {
                    github_login: "user-2".into(),
                    github_user_id: 2,
                },
            )
            .await
            .unwrap()
            .user_id;

        let mut now = Utc::now();

        let rate_limiter = RateLimiter::new(db.clone(), executor.clone());

        // User 1 can access resource A two times before being rate-limited.
        rate_limiter
            .check_internal::<RateLimitA>(user_1, now)
            .await
            .unwrap();
        rate_limiter
            .check_internal::<RateLimitA>(user_1, now)
            .await
            .unwrap();
        rate_limiter
            .check_internal::<RateLimitA>(user_1, now)
            .await
            .unwrap_err();

        // User 2 can access resource A and user 1 can access resource B.
        rate_limiter
            .check_internal::<RateLimitB>(user_2, now)
            .await
            .unwrap();
        rate_limiter
            .check_internal::<RateLimitB>(user_1, now)
            .await
            .unwrap();

        // After one second, user 1 can make another request before being rate-limited again.
        now += Duration::seconds(1);
        rate_limiter
            .check_internal::<RateLimitA>(user_1, now)
            .await
            .unwrap();
        rate_limiter
            .check_internal::<RateLimitA>(user_1, now)
            .await
            .unwrap_err();

        // Ensure pending saves to the database are flushed.
        cx.run_until_parked();

        // Rate limits are reloaded from the database, so user A is still rate-limited
        // for resource A.
        let rate_limiter = RateLimiter::new(db.clone(), executor);
        rate_limiter
            .check_internal::<RateLimitA>(user_1, now)
            .await
            .unwrap_err();
    }

    struct RateLimitA;

    impl RateLimit for RateLimitA {
        fn capacity() -> usize {
            2
        }

        fn refill_duration() -> Duration {
            Duration::seconds(2)
        }

        fn db_name() -> &'static str {
            "rate-limit-a"
        }
    }

    struct RateLimitB;

    impl RateLimit for RateLimitB {
        fn capacity() -> usize {
            10
        }

        fn refill_duration() -> Duration {
            Duration::seconds(3)
        }

        fn db_name() -> &'static str {
            "rate-limit-b"
        }
    }
}