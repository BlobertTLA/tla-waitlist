use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::RwLock;

#[derive(Debug, Clone, Default)]
pub struct WarCache {
    pub corporations: HashSet<i64>,
    pub alliances: HashSet<i64>,
}

impl WarCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contains_corporation(&self, corporation_id: i64) -> bool {
        self.corporations.contains(&corporation_id)
    }

    pub fn contains_alliance(&self, alliance_id: i64) -> bool {
        self.alliances.contains(&alliance_id)
    }

    pub fn is_at_war(&self, corporation_id: Option<i64>, alliance_id: Option<i64>) -> bool {
        if let Some(corp_id) = corporation_id {
            if self.contains_corporation(corp_id) {
                return true;
            }
        }
        if let Some(alliance_id) = alliance_id {
            if self.contains_alliance(alliance_id) {
                return true;
            }
        }
        false
    }

    pub fn replace(&mut self, corporations: HashSet<i64>, alliances: HashSet<i64>) {
        self.corporations = corporations;
        self.alliances = alliances;
    }
}

pub type SharedWarCache = Arc<RwLock<WarCache>>;

pub fn new_shared_cache() -> SharedWarCache {
    Arc::new(RwLock::new(WarCache::new()))
}

/// Collect corporation and alliance IDs from a war detail.
pub fn collect_entities_from_war(
    war: &crate::core::esi::WarDetail,
) -> (HashSet<i64>, HashSet<i64>) {
    let mut corporations = HashSet::new();
    let mut alliances = HashSet::new();

    let mut add_entity = |entity: &crate::core::esi::WarEntity| {
        if let Some(corp_id) = entity.corporation_id {
            corporations.insert(corp_id);
        }
        if let Some(alliance_id) = entity.alliance_id {
            alliances.insert(alliance_id);
        }
    };

    add_entity(&war.aggressor);
    add_entity(&war.defender);
    for ally in &war.allies {
        add_entity(ally);
    }

    (corporations, alliances)
}

/// Refresh affiliation and check whether the character's corp/alliance is in an active war.
pub async fn character_is_at_war(
    app: &crate::app::Application,
    character_id: i64,
) -> Result<bool, crate::util::madness::Madness> {
    app.affiliation_service
        .update_character_affiliation(character_id)
        .await?;

    let row = sqlx::query!(
        r#"
        SELECT c.corporation_id AS corporation_id, corp.alliance_id AS alliance_id
        FROM `character` c
        LEFT JOIN corporation corp ON corp.id = c.corporation_id
        WHERE c.id = ?
        "#,
        character_id
    )
    .fetch_optional(app.get_db())
    .await?;

    let Some(row) = row else {
        return Ok(false);
    };

    let cache = app.war_cache.read().await;
    Ok(cache.is_at_war(row.corporation_id, row.alliance_id))
}

/// DB-only war check (no ESI). Used by fleetcomp for members already in the DB.
pub async fn character_ids_at_war(
    app: &crate::app::Application,
    character_ids: &[i64],
) -> Result<HashSet<i64>, crate::util::madness::Madness> {
    let mut at_war = HashSet::new();
    if character_ids.is_empty() {
        return Ok(at_war);
    }

    let cache = app.war_cache.read().await;
    if cache.corporations.is_empty() && cache.alliances.is_empty() {
        return Ok(at_war);
    }

    for character_id in character_ids {
        let row = sqlx::query!(
            r#"
            SELECT c.corporation_id AS corporation_id, corp.alliance_id AS alliance_id
            FROM `character` c
            LEFT JOIN corporation corp ON corp.id = c.corporation_id
            WHERE c.id = ?
            "#,
            character_id
        )
        .fetch_optional(app.get_db())
        .await?;

        if let Some(row) = row {
            if cache.is_at_war(row.corporation_id, row.alliance_id) {
                at_war.insert(*character_id);
            }
        }
    }

    Ok(at_war)
}
