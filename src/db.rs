use crate::models::Job;
use sqlx::{Row, SqlitePool};

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let pool = SqlitePool::connect(url).await?;
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        )
        .execute(&pool)
        .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, body TEXT NOT NULL, created TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("UPDATE jobs SET body=json_set(body,'$.state','cancelled','$.error','Serveur redémarré pendant cette opération. Vérifiez Docker avant de relancer.') WHERE json_extract(body,'$.state') IN ('running','queued')").execute(&pool).await?;
        Ok(Self { pool })
    }
    pub async fn settings(&self) -> anyhow::Result<serde_json::Value> {
        let mut result = serde_json::Map::new();
        for row in sqlx::query("SELECT key,value FROM settings")
            .fetch_all(&self.pool)
            .await?
        {
            let value: String = row.get("value");
            result.insert(row.get("key"), serde_json::from_str(&value)?);
        }
        Ok(result.into())
    }
    pub async fn set(&self, key: &str, value: &serde_json::Value) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
            .bind(key).bind(value.to_string()).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn save_job(&self, job: &Job, limit: usize) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO jobs(id,body,created) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET body=excluded.body")
            .bind(&job.id).bind(serde_json::to_string(job)?).bind(job.created_at.to_rfc3339()).execute(&self.pool).await?;
        sqlx::query(
            "DELETE FROM jobs WHERE id NOT IN (SELECT id FROM jobs ORDER BY created DESC LIMIT ?)",
        )
        .bind(limit as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    pub async fn jobs(&self) -> anyhow::Result<Vec<Job>> {
        let rows = sqlx::query("SELECT body FROM jobs ORDER BY created DESC LIMIT 500")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| serde_json::from_str(r.get::<&str, _>("body")).map_err(Into::into))
            .collect()
    }
}
