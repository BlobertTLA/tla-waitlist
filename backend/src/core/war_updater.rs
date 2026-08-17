use crate::core::esi::{self, ESIClient, WarDetail};
use crate::data::war::{collect_entities_from_war, SharedWarCache};
use crate::{config::Config, util::madness::Madness};
use sqlx::Row;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::time::Duration;

const META_LIST_ETAG: &str = "wars_list_etag";
const META_MAX_WAR_ID: &str = "max_war_id_seen";
const META_BOOTSTRAP_DONE: &str = "bootstrap_done";

pub struct WarUpdater {
    db: Arc<crate::DB>,
    config: Config,
    esi_client: ESIClient,
    war_cache: SharedWarCache,
}

impl WarUpdater {
    pub fn new(db: Arc<crate::DB>, config: Config, war_cache: SharedWarCache) -> WarUpdater {
        WarUpdater {
            esi_client: ESIClient::new(
                db.clone(),
                config.esi.client_id.clone(),
                config.esi.client_secret.clone(),
            ),
            db,
            config,
            war_cache,
        }
    }

    pub fn start(self) {
        println!(
            "War updater starting with {} second interval",
            self.config.war_updater.interval_seconds
        );
        tokio::spawn(async move {
            self.run().await;
        });
    }

    async fn run(self) {
        if let Err(e) = self.reload_cache_from_db().await {
            error!("War updater: failed to load cache from DB: {:#?}", e);
        }

        loop {
            let sleep_time = match self.run_once().await {
                Ok(()) => {
                    println!(
                        "War updater: check completed successfully, sleeping for {} seconds",
                        self.config.war_updater.interval_seconds
                    );
                    self.config.war_updater.interval_seconds
                }
                Err(e) => {
                    error!("Error in war updater: {:#?}", e);
                    3600
                }
            };

            tokio::time::sleep(Duration::from_secs(sleep_time)).await;
        }
    }

    async fn run_once(&self) -> Result<(), Madness> {
        // Keep in-memory cache usable even if ESI war refresh fails this cycle.
        let _ = self.reload_cache_from_db().await;

        let war_result = self.refresh_wars().await;
        if let Err(ref e) = war_result {
            error!("War updater: war refresh failed: {:#?}", e);
        } else {
            let _ = self.reload_cache_from_db().await;
        }

        // Affiliation backfill must not depend on war ESI succeeding.
        if let Err(e) = self.refresh_fleet_affiliations().await {
            error!("War updater: fleet affiliation refresh failed: {:#?}", e);
            return Err(e);
        }

        war_result
    }

    async fn refresh_wars(&self) -> Result<(), Madness> {
        let bootstrap_done = self.get_meta(META_BOOTSTRAP_DONE).await?.as_deref() == Some("1");

        if !bootstrap_done {
            self.bootstrap_active_wars().await?;
            self.set_meta(META_BOOTSTRAP_DONE, "1").await?;
        } else {
            self.refresh_new_and_active_wars().await?;
        }
        Ok(())
    }

    /// Refresh corporation_id for active fleet members (bulk affiliation).
    /// Re-fetches everyone in fleet so corp/alliance changes are picked up, not only NULL corps.
    async fn refresh_fleet_affiliations(&self) -> Result<(), Madness> {
        let rows = sqlx::query(
            r#"
            SELECT DISTINCT fa.character_id
            FROM fleet_activity fa
            WHERE fa.has_left = 0
            "#,
        )
        .fetch_all(self.db.as_ref())
        .await?;

        if rows.is_empty() {
            return Ok(());
        }

        let character_ids: Vec<i64> = rows.iter().map(|row| row.get("character_id")).collect();
        println!(
            "War updater: bulk-refreshing affiliation for {} active fleet members",
            character_ids.len()
        );

        let affiliation = crate::core::affiliation::AffiliationService::new(
            self.db.clone(),
            ESIClient::new(
                self.db.clone(),
                self.config.esi.client_id.clone(),
                self.config.esi.client_secret.clone(),
            ),
        );

        affiliation
            .update_characters_affiliation_bulk(&character_ids)
            .await?;

        Ok(())
    }

    async fn bootstrap_active_wars(&self) -> Result<(), Madness> {
        println!("War updater: bootstrapping active wars from newest IDs...");
        let mut page = self.fetch_war_list_page(None).await?;

        if page.is_empty() {
            return Ok(());
        }

        let max_war_id_seen = *page.iter().max().unwrap_or(&0);

        loop {
            let mut all_finished = true;
            for war_id in &page {
                match self.fetch_and_store_war(*war_id, None).await? {
                    WarFetchResult::Active | WarFetchResult::Unchanged => {
                        all_finished = false;
                    }
                    WarFetchResult::Finished | WarFetchResult::Missing => {}
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }

            if all_finished {
                println!(
                    "War updater: bootstrap stopped after a full finished page (min_war_id={})",
                    page.iter().min().copied().unwrap_or(0)
                );
                break;
            }

            let min_id = match page.iter().min().copied() {
                Some(id) if id > 1 => id,
                _ => break,
            };

            page = self.fetch_war_list_page(Some(min_id)).await?;
            if page.is_empty() {
                break;
            }
        }

        self.set_meta(META_MAX_WAR_ID, &max_war_id_seen.to_string())
            .await?;
        Ok(())
    }

    async fn refresh_new_and_active_wars(&self) -> Result<(), Madness> {
        let list_etag = self.get_meta(META_LIST_ETAG).await?;
        let max_seen: i64 = self
            .get_meta(META_MAX_WAR_ID)
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        let list_response = self.esi_client.get_wars(list_etag.as_deref()).await?;
        if let Some(etag) = &list_response.etag {
            self.set_meta(META_LIST_ETAG, etag).await?;
        }

        let mut new_max = max_seen;

        if let Some(war_ids) = list_response.data {
            let mut to_fetch: Vec<i64> = war_ids
                .iter()
                .copied()
                .filter(|id| *id > max_seen)
                .collect();

            if let Some(min_on_page) = war_ids.iter().min().copied() {
                let mut cursor = min_on_page;
                while cursor > max_seen + 1 {
                    let next_page = self.esi_client.get_wars_page(cursor).await?;
                    if next_page.is_empty() {
                        break;
                    }
                    let new_on_page: Vec<i64> = next_page
                        .iter()
                        .copied()
                        .filter(|id| *id > max_seen)
                        .collect();
                    if new_on_page.is_empty() {
                        break;
                    }
                    to_fetch.extend(new_on_page);
                    cursor = match next_page.iter().min().copied() {
                        Some(id) if id > 1 => id,
                        _ => break,
                    };
                }
            }

            for war_id in to_fetch {
                self.fetch_and_store_war(war_id, None).await?;
                if war_id > new_max {
                    new_max = war_id;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }

        let active_rows = sqlx::query("SELECT id, etag FROM active_war")
            .fetch_all(self.db.as_ref())
            .await?;

        for row in active_rows {
            let id: i64 = row.get("id");
            let etag: Option<String> = row.get("etag");
            self.fetch_and_store_war(id, etag.as_deref()).await?;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        if new_max > max_seen {
            self.set_meta(META_MAX_WAR_ID, &new_max.to_string()).await?;
        }

        Ok(())
    }

    async fn fetch_war_list_page(&self, max_war_id: Option<i64>) -> Result<Vec<i64>, Madness> {
        let ids = match max_war_id {
            Some(max_id) => self.esi_client.get_wars_page(max_id).await?,
            None => {
                let response = self.esi_client.get_wars(None).await?;
                if let Some(etag) = response.etag {
                    self.set_meta(META_LIST_ETAG, &etag).await?;
                }
                response.data.unwrap_or_default()
            }
        };
        Ok(ids)
    }

    async fn fetch_and_store_war(
        &self,
        war_id: i64,
        etag: Option<&str>,
    ) -> Result<WarFetchResult, Madness> {
        let response = match self.esi_client.get_war(war_id, etag).await {
            Ok(r) => r,
            Err(esi::ESIError::WithMessage(status, _)) if status == 404 => {
                self.delete_war(war_id).await?;
                return Ok(WarFetchResult::Missing);
            }
            Err(e) => return Err(e.into()),
        };

        if response.data.is_none() {
            return Ok(WarFetchResult::Unchanged);
        }

        let war = response.data.unwrap();
        let now = chrono::Utc::now().timestamp();
        let new_etag = response.etag;

        if war.finished.is_some() {
            self.delete_war(war_id).await?;
            return Ok(WarFetchResult::Finished);
        }

        self.upsert_active_war(&war, new_etag.as_deref(), now).await?;
        Ok(WarFetchResult::Active)
    }

    async fn upsert_active_war(
        &self,
        war: &WarDetail,
        etag: Option<&str>,
        now: i64,
    ) -> Result<(), Madness> {
        sqlx::query(
            "INSERT INTO active_war (id, etag, updated_at) VALUES (?, ?, ?)
             ON DUPLICATE KEY UPDATE etag = VALUES(etag), updated_at = VALUES(updated_at)",
        )
        .bind(war.id)
        .bind(etag)
        .bind(now)
        .execute(self.db.as_ref())
        .await?;

        sqlx::query("DELETE FROM war_participant WHERE war_id = ?")
            .bind(war.id)
            .execute(self.db.as_ref())
            .await?;

        let (corps, allis) = collect_entities_from_war(war);
        for corp_id in corps {
            sqlx::query(
                "INSERT IGNORE INTO war_participant (war_id, entity_id, category) VALUES (?, ?, 'corporation')",
            )
            .bind(war.id)
            .bind(corp_id)
            .execute(self.db.as_ref())
            .await?;
        }
        for alliance_id in allis {
            sqlx::query(
                "INSERT IGNORE INTO war_participant (war_id, entity_id, category) VALUES (?, ?, 'alliance')",
            )
            .bind(war.id)
            .bind(alliance_id)
            .execute(self.db.as_ref())
            .await?;
        }

        Ok(())
    }

    async fn delete_war(&self, war_id: i64) -> Result<(), Madness> {
        sqlx::query("DELETE FROM war_participant WHERE war_id = ?")
            .bind(war_id)
            .execute(self.db.as_ref())
            .await?;
        sqlx::query("DELETE FROM active_war WHERE id = ?")
            .bind(war_id)
            .execute(self.db.as_ref())
            .await?;
        Ok(())
    }

    async fn reload_cache_from_db(&self) -> Result<(), Madness> {
        let rows = sqlx::query("SELECT DISTINCT entity_id, category FROM war_participant")
            .fetch_all(self.db.as_ref())
            .await?;

        let mut corporations = HashSet::new();
        let mut alliances = HashSet::new();
        for row in rows {
            let entity_id: i64 = row.get("entity_id");
            let category: String = row.get("category");
            match category.as_str() {
                "corporation" => {
                    corporations.insert(entity_id);
                }
                "alliance" => {
                    alliances.insert(entity_id);
                }
                _ => {}
            }
        }

        let corp_count = corporations.len();
        let alliance_count = alliances.len();

        {
            let mut cache = self.war_cache.write().await;
            cache.replace(corporations, alliances);
        }

        println!(
            "War updater: cache now has {} corporations and {} alliances at war",
            corp_count, alliance_count
        );

        Ok(())
    }

    async fn get_meta(&self, key: &str) -> Result<Option<String>, Madness> {
        let row = sqlx::query("SELECT meta_value FROM war_meta WHERE meta_key = ?")
            .bind(key)
            .fetch_optional(self.db.as_ref())
            .await?;
        Ok(row.map(|r| {
            let value: String = r.get("meta_value");
            value
        }))
    }

    async fn set_meta(&self, key: &str, value: &str) -> Result<(), Madness> {
        sqlx::query(
            "INSERT INTO war_meta (meta_key, meta_value) VALUES (?, ?)
             ON DUPLICATE KEY UPDATE meta_value = VALUES(meta_value)",
        )
        .bind(key)
        .bind(value)
        .execute(self.db.as_ref())
        .await?;
        Ok(())
    }
}

enum WarFetchResult {
    Active,
    Finished,
    Unchanged,
    Missing,
}
