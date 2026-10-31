use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use crate::config::SeedEntry;
use crate::models::{
    AegisDeepfakeResult, AttributionAct, Claim, ClaimWithEvidence, Creator, Decision,
    EntityIntelEvent, Evidence, HardwareProfile, Job, LeadCardSummary, OcrText, Person,
    ProviderUsageSummary, SanctionsMatch, Seed, Stage, Transcript, TranscriptSegment, Video,
    VideoDetail, VideoRef, VideoState,
};
use crate::politeness::compute_backoff_delay;

const INITIAL_SCHEMA_SQL: &str = include_str!("../migrations/0001_initial.sql");

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Default)]
pub struct VideoFilter {
    pub lane: Option<String>,
    pub state: Option<String>,
    pub verdict: Option<String>,
    pub label: Option<String>,
    pub creator_id: Option<String>,
    pub search_query: Option<String>,
    pub min_score: Option<f64>,
    pub min_views: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

impl Database {
    pub async fn connect(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let dsn = format!("sqlite://{}?mode=rwc", db_path.to_string_lossy().replace('\\', "/"));
        let opts = SqliteConnectOptions::from_str(&dsn)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(15));

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await?;

        sqlx::raw_sql(INITIAL_SCHEMA_SQL).execute(&pool).await?;

        let migrations = [
            "ALTER TABLE people ADD COLUMN tags_json TEXT DEFAULT '[]'",
            "ALTER TABLE people ADD COLUMN rating INTEGER DEFAULT 50",
            "ALTER TABLE people ADD COLUMN category TEXT",
            "ALTER TABLE people ADD COLUMN notes TEXT",
            "ALTER TABLE people ADD COLUMN detailed_profile_json TEXT",
            "ALTER TABLE people ADD COLUMN entity_type TEXT DEFAULT 'person'",
            "ALTER TABLE creators ADD COLUMN risk_assessment_json TEXT",
            "CREATE TABLE IF NOT EXISTS aegis_deepfakes (video_id TEXT PRIMARY KEY REFERENCES videos(id) ON DELETE CASCADE, payload_json TEXT NOT NULL, overall_verdict TEXT NOT NULL, audio_score REAL NOT NULL, video_score REAL NOT NULL, target_person TEXT, analyzed_at TEXT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS entity_intel_events (id INTEGER PRIMARY KEY AUTOINCREMENT, person_id INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE, source_name TEXT NOT NULL, title TEXT NOT NULL, url TEXT NOT NULL, discovered_at TEXT NOT NULL, snippet TEXT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS entity_sanctions_cache (person_id INTEGER PRIMARY KEY REFERENCES people(id) ON DELETE CASCADE, payload_json TEXT NOT NULL, checked_at TEXT NOT NULL)",
            "ALTER TABLE people ADD COLUMN last_monitored_at TEXT",
            "CREATE TABLE IF NOT EXISTS claim_verification_feedback (id INTEGER PRIMARY KEY AUTOINCREMENT, claim_id INTEGER NOT NULL, video_id TEXT NOT NULL, rating TEXT NOT NULL, suggested_verdict TEXT, notes TEXT, created_at TEXT NOT NULL)",
        ];
        for mig in migrations {
            let _ = sqlx::raw_sql(mig).execute(&pool).await;
        }

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn journal_mode(&self) -> Result<String> {
        let row = sqlx::query("PRAGMA journal_mode;").fetch_one(&self.pool).await?;
        Ok(row.get::<String, _>(0))
    }

    pub async fn seed_defaults(&self, entries: &[SeedEntry]) -> Result<usize> {
        let mut inserted = 0usize;
        for entry in entries {
            let res = sqlx::query(
                "INSERT OR IGNORE INTO seeds (kind, value, topic, active) VALUES (?1, ?2, ?3, 1)",
            )
            .bind(&entry.kind)
            .bind(&entry.value)
            .bind(&entry.topic)
            .execute(&self.pool)
            .await?;
            inserted += res.rows_affected() as usize;
        }
        Ok(inserted)
    }

    pub async fn upsert_creator_from_ref(&self, vref: &VideoRef, expansion_depth: i64) -> Result<Option<String>> {
        let handle = match vref.creator_handle.as_deref() {
            Some(h) if !h.trim().is_empty() => h.trim().trim_start_matches('@').to_string(),
            _ => return Ok(None),
        };
        let creator_id = vref
            .creator_id
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| handle.to_lowercase());
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO creators (id, handle, display_name, region, follower_count, first_seen_at, expansion_depth)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
               handle = excluded.handle,
               display_name = COALESCE(excluded.display_name, creators.display_name),
               region = COALESCE(excluded.region, creators.region),
               follower_count = COALESCE(excluded.follower_count, creators.follower_count)",
        )
        .bind(&creator_id)
        .bind(&handle)
        .bind(&vref.creator_display_name)
        .bind(&vref.creator_region)
        .bind(vref.creator_follower_count)
        .bind(&now)
        .bind(expansion_depth)
        .execute(&self.pool)
        .await?;

        Ok(Some(creator_id))
    }

    pub async fn upsert_video_discovered(&self, vref: &VideoRef, expansion_depth: i64) -> Result<(Video, bool)> {
        let creator_id = self.upsert_creator_from_ref(vref, expansion_depth).await?;
        let existing = self.get_video(&vref.id).await?;
        let now = Utc::now().to_rfc3339();
        let hashtags_json = serde_json::to_string(&vref.hashtags)?;

        if let Some(prev) = existing {
            let promoted_lane = if vref.discovered_via.starts_with("paste") {
                Some("pasted")
            } else {
                prev.lane.as_deref()
            };
            sqlx::query(
                "UPDATE videos SET
                   creator_id = COALESCE(?2, creator_id),
                   caption = COALESCE(?3, caption),
                   hashtags = CASE WHEN ?4 != '[]' THEN ?4 ELSE hashtags END,
                   posted_at = COALESCE(?5, posted_at),
                   views = COALESCE(?6, views),
                   likes = COALESCE(?7, likes),
                   shares = COALESCE(?8, shares),
                   comments = COALESCE(?9, comments),
                   duration_s = COALESCE(?10, duration_s),
                   lane = COALESCE(?11, lane)
                 WHERE id = ?1",
            )
            .bind(&vref.id)
            .bind(&creator_id)
            .bind(&vref.caption)
            .bind(&hashtags_json)
            .bind(&vref.posted_at)
            .bind(vref.views)
            .bind(vref.likes)
            .bind(vref.shares)
            .bind(vref.comments)
            .bind(vref.duration_s)
            .bind(promoted_lane)
            .execute(&self.pool)
            .await?;

            let updated = self.get_video(&vref.id).await?.unwrap_or(prev);
            return Ok((updated, false));
        }

        let initial_lane = if vref.discovered_via.starts_with("paste") {
            Some("pasted")
        } else {
            None
        };
        sqlx::query(
            "INSERT INTO videos (
                id, creator_id, url, caption, hashtags, posted_at, views, likes, shares, comments,
                duration_s, discovered_via, discovered_at, state, lane
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        )
        .bind(&vref.id)
        .bind(&creator_id)
        .bind(&vref.url)
        .bind(&vref.caption)
        .bind(&hashtags_json)
        .bind(&vref.posted_at)
        .bind(vref.views)
        .bind(vref.likes)
        .bind(vref.shares)
        .bind(vref.comments)
        .bind(vref.duration_s)
        .bind(&vref.discovered_via)
        .bind(&now)
        .bind(VideoState::Discovered.as_str())
        .bind(initial_lane)
        .execute(&self.pool)
        .await?;

        let inserted = self
            .get_video(&vref.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("inserted video missing: {}", vref.id))?;
        Ok((inserted, true))
    }

    fn map_video_row(row: &SqliteRow) -> Video {
        let hashtags_raw: Option<String> = row.try_get("hashtags").ok().flatten();
        let hashtags = hashtags_raw
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .unwrap_or_default();

        Video {
            id: row.get("id"),
            creator_id: row.try_get("creator_id").ok().flatten(),
            creator_handle: row.try_get("creator_handle").ok().flatten(),
            creator_display_name: row.try_get("creator_display_name").ok().flatten(),
            url: row.get("url"),
            caption: row.try_get("caption").ok().flatten(),
            hashtags,
            posted_at: row.try_get("posted_at").ok().flatten(),
            views: row.try_get("views").ok().flatten(),
            likes: row.try_get("likes").ok().flatten(),
            shares: row.try_get("shares").ok().flatten(),
            comments: row.try_get("comments").ok().flatten(),
            duration_s: row.try_get("duration_s").ok().flatten(),
            discovered_via: row.get("discovered_via"),
            discovered_at: row.get("discovered_at"),
            phash: row.try_get("phash").ok().flatten(),
            state: row.get("state"),
            state_reason: row.try_get("state_reason").ok().flatten(),
            media_path: row.try_get("media_path").ok().flatten(),
            lead_score: row.try_get("lead_score").ok().flatten(),
            lane: row.try_get("lane").ok().flatten(),
            cluster_id: row.try_get("cluster_id").ok().flatten(),
        }
    }

    pub async fn get_video(&self, video_id: &str) -> Result<Option<Video>> {
        let row = sqlx::query(
            "SELECT v.*, c.handle AS creator_handle, c.display_name AS creator_display_name
             FROM videos v
             LEFT JOIN creators c ON c.id = v.creator_id
             WHERE v.id = ?1",
        )
        .bind(video_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.as_ref().map(Self::map_video_row))
    }

    pub async fn set_video_state(&self, video_id: &str, state: VideoState, reason: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE videos SET state = ?2, state_reason = ?3 WHERE id = ?1")
            .bind(video_id)
            .bind(state.as_str())
            .bind(reason)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_video_media_and_phash(&self, video_id: &str, media_path: Option<&str>, phash: Option<&str>) -> Result<()> {
        sqlx::query(
            "UPDATE videos SET
               media_path = COALESCE(?2, media_path),
               phash = COALESCE(?3, phash)
             WHERE id = ?1",
        )
        .bind(video_id)
        .bind(media_path)
        .bind(phash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn clear_video_media_path(&self, video_id: &str) -> Result<()> {
        sqlx::query("UPDATE videos SET media_path = NULL WHERE id = ?1")
            .bind(video_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_known_phashes(&self) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT phash FROM videos WHERE phash IS NOT NULL")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.get::<Option<String>, _>("phash"))
            .collect())
    }

    pub async fn set_video_ranking(
        &self,
        video_id: &str,
        lead_score: f64,
        lane: &str,
        cluster_id: Option<i64>,
        state: VideoState,
        reason: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE videos SET
               lead_score = ?2,
               lane = ?3,
               cluster_id = COALESCE(?4, cluster_id),
               state = ?5,
               state_reason = ?6
             WHERE id = ?1",
        )
        .bind(video_id)
        .bind(lead_score)
        .bind(lane)
        .bind(cluster_id)
        .bind(state.as_str())
        .bind(reason)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_video_lane(&self, video_id: &str, lane: &str) -> Result<()> {
        sqlx::query("UPDATE videos SET lane = ?2 WHERE id = ?1")
            .bind(video_id)
            .bind(lane)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_videos(&self, filter: &VideoFilter) -> Result<Vec<Video>> {
        let limit = filter.limit.unwrap_or(200).clamp(1, 5000);
        let offset = filter.offset.unwrap_or(0).max(0);
        let rows = sqlx::query(
            "SELECT v.*, c.handle AS creator_handle, c.display_name AS creator_display_name
             FROM videos v
             LEFT JOIN creators c ON c.id = v.creator_id
             WHERE (?1 IS NULL
                    OR (?1 = 'pasted' AND v.lane = 'pasted')
                    OR (?1 = 'leads' AND (v.lane = 'leads' OR (v.lane IS NULL AND v.state IN ('VERIFIED', 'READY'))))
                    OR (?1 = 'already_checked' AND (v.lane = 'already_checked' OR EXISTS (SELECT 1 FROM claims cl WHERE cl.video_id = v.id AND (cl.verdict = 'ALREADY_CHECKED' OR cl.verdict = 'already_checked'))))
                    OR v.lane = ?1)
               AND (?2 IS NULL OR v.state = ?2)
               AND (?1 = 'discarded' OR ?2 = 'DISCARDED' OR (v.state != 'DISCARDED' AND COALESCE(v.lane, '') != 'discarded'))
               AND (v.creator_id IS NULL OR v.creator_id != 'creator_soak')
               AND v.id NOT LIKE 'rv%'
               AND (?3 IS NULL OR v.creator_id = ?3 OR c.handle = ?3 OR c.handle = REPLACE(?3, '@', ''))
               AND (?4 IS NULL OR COALESCE(v.lead_score, 0.0) >= ?4)
               AND (?5 IS NULL OR COALESCE(v.views, 0) >= ?5)
               AND (?6 IS NULL OR LOWER(COALESCE(v.caption, '')) LIKE '%' || LOWER(?6) || '%'
                    OR LOWER(COALESCE(c.handle, '')) LIKE '%' || LOWER(?6) || '%'
                    OR EXISTS (SELECT 1 FROM claims cl WHERE cl.video_id = v.id AND LOWER(cl.text_bg) LIKE '%' || LOWER(?6) || '%'))
               AND (?7 IS NULL OR EXISTS (SELECT 1 FROM claims cl WHERE cl.video_id = v.id AND cl.verdict = ?7))
               AND (?8 IS NULL OR EXISTS (SELECT 1 FROM claims cl WHERE cl.video_id = v.id AND cl.label = ?8))
             ORDER BY
                CASE WHEN v.discovered_via LIKE 'paste%' OR v.lane = 'pasted' THEN 1000.0 ELSE 0.0 END +
                COALESCE(v.lead_score, -1.0) DESC,
                v.discovered_at DESC
             LIMIT ?9 OFFSET ?10",
        )
        .bind(&filter.lane)
        .bind(&filter.state)
        .bind(&filter.creator_id)
        .bind(filter.min_score)
        .bind(filter.min_views)
        .bind(&filter.search_query)
        .bind(&filter.verdict)
        .bind(&filter.label)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.iter().map(Self::map_video_row).collect())
    }

    pub async fn get_lane_counts(&self) -> Result<std::collections::HashMap<String, i64>> {
        let rows = sqlx::query(
            "SELECT lane, COUNT(*) AS cnt
             FROM videos
             WHERE lane IS NOT NULL
             GROUP BY lane",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut map = std::collections::HashMap::new();
        for r in rows {
            let k: String = r.get("lane");
            let c: i64 = r.get("cnt");
            map.insert(k, c);
        }
        let leads_cnt_row = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM videos WHERE lane = 'leads' OR (lane IS NULL AND state IN ('VERIFIED', 'READY'))",
        )
        .fetch_one(&self.pool)
        .await?;
        let leads_cnt: i64 = leads_cnt_row.get("cnt");
        map.insert("leads".to_string(), leads_cnt);

        let already_checked_cnt_row = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM videos WHERE lane = 'already_checked' OR EXISTS (SELECT 1 FROM claims cl WHERE cl.video_id = videos.id AND (cl.verdict = 'ALREADY_CHECKED' OR cl.verdict = 'already_checked'))",
        )
        .fetch_one(&self.pool)
        .await?;
        let already_checked_cnt: i64 = already_checked_cnt_row.get("cnt");
        map.insert("already_checked".to_string(), already_checked_cnt);

        let pasted_cnt_row = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM videos WHERE lane = 'pasted'",
        )
        .fetch_one(&self.pool)
        .await?;
        let pasted_cnt: i64 = pasted_cnt_row.get("cnt");
        map.insert("pasted".to_string(), pasted_cnt);
        Ok(map)
    }

    pub async fn list_lead_cards(&self, filter: &VideoFilter) -> Result<Vec<LeadCardSummary>> {
        let videos = self.list_videos(filter).await?;
        let mut cards = Vec::with_capacity(videos.len());

        for video in videos {
            let tr_row = sqlx::query("SELECT source, quality FROM transcripts WHERE video_id = ?1")
                .bind(&video.id)
                .fetch_optional(&self.pool)
                .await?;
            let (transcript_source, transcript_quality) = match tr_row {
                Some(r) => (
                    r.try_get::<Option<String>, _>("source").ok().flatten(),
                    r.try_get::<Option<f64>, _>("quality").ok().flatten(),
                ),
                None => (None, None),
            };

            let top_claim_row = sqlx::query(
                "SELECT text_bg, verdict, label, checkworthiness, evidence_strength
                 FROM claims
                 WHERE video_id = ?1
                 ORDER BY COALESCE(checkworthiness, 0.0) DESC, id ASC
                 LIMIT 1",
            )
            .bind(&video.id)
            .fetch_optional(&self.pool)
            .await?;

            let (top_claim_bg, top_verdict, top_label, max_checkworthiness, max_evidence_strength) =
                match top_claim_row {
                    Some(r) => (
                        r.try_get::<Option<String>, _>("text_bg").ok().flatten(),
                        r.try_get::<Option<String>, _>("verdict").ok().flatten(),
                        r.try_get::<Option<String>, _>("label").ok().flatten(),
                        r.try_get::<Option<f64>, _>("checkworthiness").ok().flatten(),
                        r.try_get::<Option<f64>, _>("evidence_strength").ok().flatten(),
                    ),
                    None => (None, None, None, None, None),
                };

            let (cluster_size, cluster_total_views) = if let Some(cid) = video.cluster_id {
                let crow = sqlx::query(
                    "SELECT COUNT(*) AS cnt, COALESCE(SUM(COALESCE(views, 0)), 0) AS sum_views
                     FROM videos WHERE cluster_id = ?1",
                )
                .bind(cid)
                .fetch_one(&self.pool)
                .await?;
                (
                    crow.get::<i64, _>("cnt").max(1),
                    crow.get::<i64, _>("sum_views"),
                )
            } else {
                (1, video.views.unwrap_or(0))
            };

            let research_running_row = sqlx::query(
                "SELECT COUNT(*) AS cnt FROM jobs
                 WHERE video_id = ?1 AND status IN ('queued', 'running')",
            )
            .bind(&video.id)
            .fetch_one(&self.pool)
            .await?;
            let is_completed = matches!(video.state.as_str(), "VERIFIED" | "READY" | "KEPT" | "FAILED" | "SCREENED_OUT" | "DISCARDED");
            let research_running = research_running_row.get::<i64, _>("cnt") > 0 || !is_completed;

            let top_claim_bg_final = if top_claim_bg.is_some() {
                top_claim_bg
            } else if !is_completed {
                Some("Видеото се обработва в момента (транскрибиране, OCR и проверка)...".to_string())
            } else {
                None
            };

            let top_verdict_final = if top_verdict.is_some() {
                top_verdict
            } else if !is_completed {
                Some("PROCESSING".to_string())
            } else {
                None
            };

            let top_label_final = if top_label.is_some() {
                top_label
            } else if !is_completed {
                Some("В обработка...".to_string())
            } else {
                None
            };

            let reason_flagged = video
                .state_reason
                .clone()
                .or_else(|| top_claim_bg_final.clone())
                .unwrap_or_else(|| video.discovered_via.clone());

            let latest_job = sqlx::query(
                "SELECT stage, status FROM jobs WHERE video_id = ?1 ORDER BY id DESC LIMIT 1",
            )
            .bind(&video.id)
            .fetch_optional(&self.pool)
            .await?;

            let (progress_pct, etr_seconds, stage_desc) = if research_running_row.get::<i64, _>("cnt") > 0 && is_completed {
                (Some(75), Some(5), Some("Задълбочено проучване...".to_string()))
            } else {
                match video.state.as_str() {
                    "VERIFIED" | "READY" | "KEPT" => (Some(100), Some(0), Some("100% Завършено".to_string())),
                    "FAILED" => (Some(100), Some(0), Some("Грешка при обработка".to_string())),
                    "SCREENED_OUT" => (Some(100), Some(0), Some("Филтрирано".to_string())),
                    "DISCARDED" => (Some(100), Some(0), Some("Отхвърлено".to_string())),
                    _ => match latest_job.as_ref().and_then(|r| r.try_get::<String, _>("stage").ok()).as_deref() {
                        Some("DISCOVER") => (Some(15), Some(25), Some("Откриване...".to_string())),
                        Some("SCREEN_CHEAP") => (Some(30), Some(20), Some("Предварителен преглед...".to_string())),
                        Some("ACQUIRE") => (Some(50), Some(15), Some("Сваляне на съдържание...".to_string())),
                        Some("EXTRACT") => (Some(70), Some(10), Some("Whisper ASR + OCR...".to_string())),
                        Some("CLAIMS") | Some("CHECKWORTHY") => (Some(85), Some(5), Some("BgGPT анализ на твърдения...".to_string())),
                        Some("VERIFY") => (Some(95), Some(2), Some("Финален факт-чек...".to_string())),
                        Some("RANK") => (Some(98), Some(1), Some("Класиране...".to_string())),
                        _ => {
                            if video.state == "ACQUIRED" {
                                (Some(50), Some(15), Some("Свалено съдържание...".to_string()))
                            } else if video.state == "EXTRACTED" {
                                (Some(70), Some(10), Some("Извлечен говор...".to_string()))
                            } else if video.state == "CLAIMS_DONE" {
                                (Some(85), Some(5), Some("Извлечени твърдения...".to_string()))
                            } else if research_running {
                                (Some(50), Some(15), Some("В обработка...".to_string()))
                            } else {
                                (Some(20), Some(20), Some("В опашка...".to_string()))
                            }
                        }
                    }
                }
            };

            let aegis_row = sqlx::query(
                "SELECT overall_verdict, audio_score, video_score, target_person FROM aegis_deepfakes WHERE video_id = ?1",
            )
            .bind(&video.id)
            .fetch_optional(&self.pool)
            .await?;
            let (aegis_verdict, aegis_audio_score, aegis_video_score, aegis_target_person) = match aegis_row {
                Some(arow) => (
                    arow.try_get::<String, _>("overall_verdict").ok(),
                    arow.try_get::<f64, _>("audio_score").ok(),
                    arow.try_get::<f64, _>("video_score").ok(),
                    arow.try_get::<String, _>("target_person").ok(),
                ),
                None => (None, None, None, None),
            };

            cards.push(LeadCardSummary {
                video,
                top_claim_bg: top_claim_bg_final,
                top_verdict: top_verdict_final,
                top_label: top_label_final,
                transcript_quality,
                transcript_source,
                max_checkworthiness,
                max_evidence_strength,
                cluster_size,
                cluster_total_views,
                research_running,
                reason_flagged,
                progress_pct,
                etr_seconds,
                stage_desc,
                aegis_verdict,
                aegis_audio_score,
                aegis_video_score,
                aegis_target_person,
            });
        }

        Ok(cards)
    }

    pub async fn get_video_detail(&self, video_id: &str) -> Result<Option<VideoDetail>> {
        let Some(video) = self.get_video(video_id).await? else {
            return Ok(None);
        };

        let creator = if let Some(cid) = &video.creator_id {
            self.get_creator(cid).await?
        } else {
            None
        };

        let transcript = self.get_transcript(video_id).await?;
        let ocr_texts = self.get_ocr_texts(video_id).await?;
        let claims_raw = self.get_claims_for_video(video_id).await?;
        let mut claims = Vec::with_capacity(claims_raw.len());
        for claim in claims_raw {
            let evidence = self.get_evidence_for_claim(claim.id).await?;
            claims.push(ClaimWithEvidence { claim, evidence });
        }
        let people = self.get_people_for_video(video_id).await?;
        let decisions = self.get_decisions_for_video(video_id).await?;
        let aegis_deepfake = self.get_aegis_result(video_id).await.unwrap_or(None);

        Ok(Some(VideoDetail {
            video,
            creator,
            transcript,
            ocr_texts,
            claims,
            people,
            decisions,
            aegis_deepfake,
        }))
    }

    pub async fn save_aegis_result(&self, video_id: &str, result: &AegisDeepfakeResult) -> Result<()> {
        let payload = serde_json::to_string(result)?;
        let now = Utc::now().to_rfc3339();
        let target_person = result.audio.target_person.as_deref().or(result.video.target_person.as_deref());
        sqlx::query(
            "INSERT INTO aegis_deepfakes (video_id, payload_json, overall_verdict, audio_score, video_score, target_person, analyzed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(video_id) DO UPDATE SET
                payload_json = excluded.payload_json,
                overall_verdict = excluded.overall_verdict,
                audio_score = excluded.audio_score,
                video_score = excluded.video_score,
                target_person = excluded.target_person,
                analyzed_at = excluded.analyzed_at",
        )
        .bind(video_id)
        .bind(&payload)
        .bind(&result.overall_verdict)
        .bind(result.audio.score)
        .bind(result.video.score)
        .bind(target_person)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_aegis_result(&self, video_id: &str) -> Result<Option<AegisDeepfakeResult>> {
        let row = sqlx::query("SELECT payload_json FROM aegis_deepfakes WHERE video_id = ?1")
            .bind(video_id)
            .fetch_optional(&self.pool)
            .await?;
        if let Some(r) = row {
            let json_str: String = r.get("payload_json");
            let res = serde_json::from_str::<AegisDeepfakeResult>(&json_str)?;
            Ok(Some(res))
        } else {
            Ok(None)
        }
    }

    pub async fn get_creator(&self, creator_id: &str) -> Result<Option<Creator>> {
        let row = sqlx::query("SELECT * FROM creators WHERE id = ?1")
            .bind(creator_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| Creator {
            id: r.get("id"),
            handle: r.get("handle"),
            display_name: r.try_get("display_name").ok().flatten(),
            region: r.try_get("region").ok().flatten(),
            follower_count: r.try_get("follower_count").ok().flatten(),
            first_seen_at: r.get("first_seen_at"),
            watchlist: r.get::<i64, _>("watchlist") != 0,
            flagged_count: r.get("flagged_count"),
            expansion_depth: r.get("expansion_depth"),
            risk_assessment_json: r.try_get("risk_assessment_json").ok().flatten(),
        }))
    }

    pub async fn list_creators(&self) -> Result<Vec<Creator>> {
        let rows = sqlx::query("SELECT * FROM creators ORDER BY watchlist DESC, flagged_count DESC, handle ASC")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| Creator {
                id: r.get("id"),
                handle: r.get("handle"),
                display_name: r.try_get("display_name").ok().flatten(),
                region: r.try_get("region").ok().flatten(),
                follower_count: r.try_get("follower_count").ok().flatten(),
                first_seen_at: r.get("first_seen_at"),
                watchlist: r.get::<i64, _>("watchlist") != 0,
                flagged_count: r.get("flagged_count"),
                expansion_depth: r.get("expansion_depth"),
                risk_assessment_json: r.try_get("risk_assessment_json").ok().flatten(),
            })
            .collect())
    }

    pub async fn update_creator_risk_assessment(&self, creator_id: &str, risk_json: &str) -> Result<()> {
        sqlx::query("UPDATE creators SET risk_assessment_json = ?1 WHERE id = ?2")
            .bind(risk_json)
            .bind(creator_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_creator_watchlist(&self, handle_or_id: &str, watchlist: bool) -> Result<Creator> {
        let clean = handle_or_id.trim().trim_start_matches('@');
        let id = clean.to_lowercase();
        let now = Utc::now().to_rfc3339();
        let wl_int = if watchlist { 1i64 } else { 0i64 };

        sqlx::query(
            "INSERT INTO creators (id, handle, first_seen_at, watchlist)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET watchlist = ?4",
        )
        .bind(&id)
        .bind(clean)
        .bind(&now)
        .bind(wl_int)
        .execute(&self.pool)
        .await?;

        self.get_creator(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("creator not found after upsert"))
    }

    pub async fn save_transcript(&self, tr: &Transcript) -> Result<()> {
        let segments_json = serde_json::to_string(&tr.segments)?;
        sqlx::query(
            "INSERT INTO transcripts (video_id, source, engine, model, language, quality, text, segments_json, edited_text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(video_id) DO UPDATE SET
               source = excluded.source,
               engine = excluded.engine,
               model = excluded.model,
               language = excluded.language,
               quality = excluded.quality,
               text = excluded.text,
               segments_json = excluded.segments_json,
               edited_text = COALESCE(excluded.edited_text, transcripts.edited_text)",
        )
        .bind(&tr.video_id)
        .bind(&tr.source)
        .bind(&tr.engine)
        .bind(&tr.model)
        .bind(&tr.language)
        .bind(tr.quality)
        .bind(&tr.text)
        .bind(&segments_json)
        .bind(&tr.edited_text)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_transcript_edited_text(&self, video_id: &str, edited_text: &str) -> Result<()> {
        sqlx::query("UPDATE transcripts SET edited_text = ?2 WHERE video_id = ?1")
            .bind(video_id)
            .bind(edited_text)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_transcript(&self, video_id: &str) -> Result<Option<Transcript>> {
        let row = sqlx::query("SELECT * FROM transcripts WHERE video_id = ?1")
            .bind(video_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| {
            let seg_raw: Option<String> = r.try_get("segments_json").ok().flatten();
            let segments = seg_raw
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<TranscriptSegment>>(s).ok())
                .unwrap_or_default();
            Transcript {
                video_id: r.get("video_id"),
                source: r.get("source"),
                engine: r.try_get("engine").ok().flatten(),
                model: r.try_get("model").ok().flatten(),
                language: r.try_get("language").ok().flatten(),
                quality: r.try_get("quality").ok().flatten(),
                text: r.get("text"),
                segments,
                edited_text: r.try_get("edited_text").ok().flatten(),
            }
        }))
    }

    pub async fn replace_ocr_texts(&self, video_id: &str, items: &[OcrText]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM ocr_texts WHERE video_id = ?1")
            .bind(video_id)
            .execute(&mut *tx)
            .await?;
        for item in items {
            sqlx::query(
                "INSERT OR REPLACE INTO ocr_texts (video_id, frame_ms, text, confidence)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(video_id)
            .bind(item.frame_ms)
            .bind(&item.text)
            .bind(item.confidence)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_ocr_texts(&self, video_id: &str) -> Result<Vec<OcrText>> {
        let rows = sqlx::query("SELECT * FROM ocr_texts WHERE video_id = ?1 ORDER BY frame_ms ASC")
            .bind(video_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| OcrText {
                video_id: r.get("video_id"),
                frame_ms: r.get("frame_ms"),
                text: r.try_get::<Option<String>, _>("text").ok().flatten().unwrap_or_default(),
                confidence: r.try_get("confidence").ok().flatten(),
            })
            .collect())
    }

    pub async fn replace_claims_for_video(&self, video_id: &str, claims: &[Claim]) -> Result<Vec<Claim>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM evidence WHERE claim_id IN (SELECT id FROM claims WHERE video_id = ?1)")
            .bind(video_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM claims WHERE video_id = ?1")
            .bind(video_id)
            .execute(&mut *tx)
            .await?;

        for c in claims {
            let attr_json = serde_json::to_string(&c.attribution)?;
            sqlx::query(
                "INSERT INTO claims (
                    video_id, text_bg, claim_type, speaker, attribution_json, start_ms, end_ms,
                    checkworthiness, verdict, label, reasoning_bg, evidence_strength, verified_at, cluster_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            )
            .bind(video_id)
            .bind(&c.text_bg)
            .bind(&c.claim_type)
            .bind(&c.speaker)
            .bind(&attr_json)
            .bind(c.start_ms)
            .bind(c.end_ms)
            .bind(c.checkworthiness)
            .bind(&c.verdict)
            .bind(&c.label)
            .bind(&c.reasoning_bg)
            .bind(c.evidence_strength)
            .bind(&c.verified_at)
            .bind(c.cluster_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.get_claims_for_video(video_id).await
    }

    pub async fn get_claims_for_video(&self, video_id: &str) -> Result<Vec<Claim>> {
        let rows = sqlx::query("SELECT * FROM claims WHERE video_id = ?1 ORDER BY id ASC")
            .bind(video_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let attr_raw: Option<String> = r.try_get("attribution_json").ok().flatten();
                let attribution = attr_raw
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<Vec<AttributionAct>>(s).ok())
                    .unwrap_or_default();
                Claim {
                    id: r.get("id"),
                    video_id: r.get("video_id"),
                    text_bg: r.get("text_bg"),
                    claim_type: r.get("claim_type"),
                    speaker: r.try_get("speaker").ok().flatten(),
                    attribution,
                    start_ms: r.try_get("start_ms").ok().flatten(),
                    end_ms: r.try_get("end_ms").ok().flatten(),
                    checkworthiness: r.try_get("checkworthiness").ok().flatten(),
                    verdict: r.try_get("verdict").ok().flatten(),
                    label: r.try_get("label").ok().flatten(),
                    reasoning_bg: r.try_get("reasoning_bg").ok().flatten(),
                    evidence_strength: r.try_get("evidence_strength").ok().flatten(),
                    verified_at: r.try_get("verified_at").ok().flatten(),
                    cluster_id: r.try_get("cluster_id").ok().flatten(),
                }
            })
            .collect())
    }

    pub async fn update_claim_text(&self, claim_id: i64, text_bg: &str) -> Result<()> {
        sqlx::query("UPDATE claims SET text_bg = ?2, verdict = NULL, reasoning_bg = NULL WHERE id = ?1")
            .bind(claim_id)
            .bind(text_bg)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_claim_verification(
        &self,
        claim_id: i64,
        verdict: &str,
        label: &str,
        reasoning_bg: &str,
        evidence_strength: f64,
        evidence_items: &[Evidence],
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE claims SET
               verdict = ?2,
               label = ?3,
               reasoning_bg = ?4,
               evidence_strength = ?5,
               verified_at = ?6
             WHERE id = ?1",
        )
        .bind(claim_id)
        .bind(verdict)
        .bind(label)
        .bind(reasoning_bg)
        .bind(evidence_strength)
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM evidence WHERE claim_id = ?1")
            .bind(claim_id)
            .execute(&mut *tx)
            .await?;

        for ev in evidence_items {
            sqlx::query(
                "INSERT INTO evidence (
                    claim_id, passage_id, source_url, source_name, source_kind, published_at, passage, stance, retrieved_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .bind(claim_id)
            .bind(&ev.passage_id)
            .bind(&ev.source_url)
            .bind(&ev.source_name)
            .bind(&ev.source_kind)
            .bind(&ev.published_at)
            .bind(&ev.passage)
            .bind(&ev.stance)
            .bind(&ev.retrieved_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_evidence_for_claim(&self, claim_id: i64) -> Result<Vec<Evidence>> {
        let rows = sqlx::query("SELECT * FROM evidence WHERE claim_id = ?1 ORDER BY id ASC")
            .bind(claim_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| Evidence {
                id: r.get("id"),
                claim_id: r.get("claim_id"),
                passage_id: r.get("passage_id"),
                source_url: r.get("source_url"),
                source_name: r.try_get("source_name").ok().flatten(),
                source_kind: r.get("source_kind"),
                published_at: r.try_get("published_at").ok().flatten(),
                passage: r.get("passage"),
                stance: r.try_get("stance").ok().flatten(),
                retrieved_at: r.get("retrieved_at"),
            })
            .collect())
    }

    pub async fn delete_evidence(&self, id: i64) -> Result<bool> {
        let res = sqlx::query("DELETE FROM evidence WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn upsert_person_for_video(
        &self,
        video_id: &str,
        canonical_name: &str,
        variants: &[String],
        role: Option<&str>,
        public_figure: bool,
        wikidata_id: Option<&str>,
        summary: Option<&str>,
        mentions: i64,
        found_in: &str,
        entity_type: Option<&str>,
    ) -> Result<i64> {
        let existing = sqlx::query("SELECT id, variants_json FROM people WHERE LOWER(canonical_name) = LOWER(?1) LIMIT 1")
            .bind(canonical_name)
            .fetch_optional(&self.pool)
            .await?;

        let now = Utc::now().to_rfc3339();
        let person_id = if let Some(row) = existing {
            let pid: i64 = row.get("id");
            let prev_vars: Vec<String> = row
                .try_get::<Option<String>, _>("variants_json")
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let mut merged = prev_vars;
            for v in variants {
                if !merged.iter().any(|x| x.eq_ignore_ascii_case(v)) {
                    merged.push(v.clone());
                }
            }
            let vars_json = serde_json::to_string(&merged)?;
            sqlx::query(
                "UPDATE people SET
                   variants_json = ?2,
                   role = COALESCE(?3, role),
                   public_figure = CASE WHEN ?4 = 1 THEN 1 ELSE public_figure END,
                   wikidata_id = COALESCE(?5, wikidata_id),
                   summary = COALESCE(?6, summary),
                   researched_at = CASE WHEN ?6 IS NOT NULL THEN ?7 ELSE researched_at END,
                   entity_type = COALESCE(?8, entity_type)
                 WHERE id = ?1",
            )
            .bind(pid)
            .bind(&vars_json)
            .bind(role)
            .bind(if public_figure { 1i64 } else { 0i64 })
            .bind(wikidata_id)
            .bind(summary)
            .bind(&now)
            .bind(entity_type)
            .execute(&self.pool)
            .await?;
            pid
        } else {
            let vars_json = serde_json::to_string(variants)?;
            let researched = if summary.is_some() { Some(now) } else { None };
            let ent = entity_type.unwrap_or("person");
            let res = sqlx::query(
                "INSERT INTO people (canonical_name, variants_json, role, public_figure, wikidata_id, summary, researched_at, entity_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .bind(canonical_name)
            .bind(&vars_json)
            .bind(role)
            .bind(if public_figure { 1i64 } else { 0i64 })
            .bind(wikidata_id)
            .bind(summary)
            .bind(researched)
            .bind(ent)
            .execute(&self.pool)
            .await?;
            res.last_insert_rowid()
        };

        sqlx::query(
            "INSERT INTO video_people (video_id, person_id, mentions, found_in)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(video_id, person_id) DO UPDATE SET
               mentions = excluded.mentions,
               found_in = excluded.found_in",
        )
        .bind(video_id)
        .bind(person_id)
        .bind(mentions.max(1))
        .bind(found_in)
        .execute(&self.pool)
        .await?;

        Ok(person_id)
    }

    pub async fn clear_people_for_video(&self, video_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM video_people WHERE video_id = ?1")
            .bind(video_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_people_for_video(&self, video_id: &str) -> Result<Vec<Person>> {
        let rows = sqlx::query(
            "SELECT p.*, vp.mentions, vp.found_in
             FROM video_people vp
             JOIN people p ON p.id = vp.person_id
             WHERE vp.video_id = ?1
             ORDER BY p.public_figure DESC, vp.mentions DESC",
        )
        .bind(video_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let vars: Vec<String> = r
                    .try_get::<Option<String>, _>("variants_json")
                    .ok()
                    .flatten()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                let tags: Vec<String> = r
                    .try_get::<Option<String>, _>("tags_json")
                    .ok()
                    .flatten()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                Person {
                    id: r.get("id"),
                    canonical_name: r.get("canonical_name"),
                    variants: vars,
                    role: r.try_get("role").ok().flatten(),
                    public_figure: r.get::<i64, _>("public_figure") != 0,
                    wikidata_id: r.try_get("wikidata_id").ok().flatten(),
                    summary: r.try_get("summary").ok().flatten(),
                    researched_at: r.try_get("researched_at").ok().flatten(),
                    mentions: r.try_get("mentions").ok(),
                    found_in: r.try_get("found_in").ok().flatten(),
                    tags,
                    rating: r.try_get::<i64, _>("rating").unwrap_or(50),
                    category: r.try_get("category").ok().flatten(),
                    notes: r.try_get("notes").ok().flatten(),
                    detailed_profile_json: r.try_get("detailed_profile_json").ok().flatten(),
                    entity_type: r.try_get::<Option<String>, _>("entity_type").ok().flatten().unwrap_or_else(|| "person".to_string()),
                }
            })
            .collect())
    }

    pub async fn log_decision(&self, video_id: &str, action: &str, note: Option<&str>) -> Result<Decision> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "INSERT INTO decisions (video_id, action, note, decided_at) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(video_id)
        .bind(action)
        .bind(note)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        let new_state = match action {
            "keep" => Some(VideoState::Kept),
            "discard" => Some(VideoState::Discarded),
            "undo" => Some(VideoState::Ready),
            _ => None,
        };
        if let Some(st) = new_state {
            self.set_video_state(video_id, st, None).await?;
            if action == "discard" {
                sqlx::query("UPDATE videos SET lane = 'discarded' WHERE id = ?1")
                    .bind(video_id)
                    .execute(&self.pool)
                    .await?;
            } else if action == "keep" {
                sqlx::query("UPDATE videos SET lane = 'kept' WHERE id = ?1")
                    .bind(video_id)
                    .execute(&self.pool)
                    .await?;
            }
        }

        Ok(Decision {
            id: res.last_insert_rowid(),
            video_id: video_id.to_string(),
            action: action.to_string(),
            note: note.map(|s| s.to_string()),
            decided_at: now,
        })
    }

    pub async fn delete_video(&self, video_id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM evidence WHERE claim_id IN (SELECT id FROM claims WHERE video_id = ?1)")
            .bind(video_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM claims WHERE video_id = ?1")
            .bind(video_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM video_people WHERE video_id = ?1")
            .bind(video_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM transcripts WHERE video_id = ?1")
            .bind(video_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM ocr_texts WHERE video_id = ?1")
            .bind(video_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM decisions WHERE video_id = ?1")
            .bind(video_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM jobs WHERE video_id = ?1")
            .bind(video_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM videos WHERE id = ?1")
            .bind(video_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn update_person_profile(
        &self,
        person_id: i64,
        role: Option<&str>,
        summary: Option<&str>,
        public_figure: bool,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE people SET
               role = COALESCE(?2, role),
               summary = COALESCE(?3, summary),
               public_figure = CASE WHEN ?4 = 1 THEN 1 ELSE public_figure END,
               researched_at = ?5
             WHERE id = ?1",
        )
        .bind(person_id)
        .bind(role)
        .bind(summary)
        .bind(if public_figure { 1i64 } else { 0i64 })
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_person_claims_stats(&self, person_id: i64) -> Result<(i64, i64, i64)> {
        let row = sqlx::query(
            "SELECT
               COUNT(c.id) AS total,
               SUM(CASE WHEN UPPER(COALESCE(c.verdict, '')) = 'CONTRADICTED' THEN 1 ELSE 0 END) AS contradicted,
               SUM(CASE WHEN UPPER(COALESCE(c.verdict, '')) = 'SUPPORTED' THEN 1 ELSE 0 END) AS supported
             FROM claims c
             JOIN video_people vp ON vp.video_id = c.video_id
             WHERE vp.person_id = ?1",
        )
        .bind(person_id)
        .fetch_one(&self.pool)
        .await?;

        let total: i64 = row.try_get("total").unwrap_or(0);
        let contradicted: i64 = row.try_get("contradicted").unwrap_or(0);
        let supported: i64 = row.try_get("supported").unwrap_or(0);
        Ok((total, contradicted, supported))
    }

    pub async fn get_videos_for_person(&self, person_id: i64) -> Result<Vec<(String, Option<String>, Option<String>)>> {
        let rows = sqlx::query(
            "SELECT v.id, v.caption, c.handle
             FROM video_people vp
             JOIN videos v ON v.id = vp.video_id
             LEFT JOIN creators c ON c.id = v.creator_id
             WHERE vp.person_id = ?1
             LIMIT 10",
        )
        .bind(person_id)
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::new();
        for r in rows {
            let id: String = r.try_get("id").unwrap_or_default();
            let caption: Option<String> = r.try_get("caption").ok();
            let handle: Option<String> = r.try_get("handle").ok();
            out.push((id, caption, handle));
        }
        Ok(out)
    }

    pub async fn list_all_people(
        &self,
        query: Option<&str>,
        tag: Option<&str>,
        category: Option<&str>,
        sort_by: Option<&str>,
        entity_type: Option<&str>,
    ) -> Result<Vec<Person>> {
        let rows = sqlx::query(
            "SELECT p.*,
                    COALESCE((SELECT SUM(vp.mentions) FROM video_people vp WHERE vp.person_id = p.id), 0) AS total_mentions
             FROM people p
             ORDER BY p.rating DESC, total_mentions DESC, p.canonical_name ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        let q_lower = query.map(|q| q.trim().to_lowercase()).filter(|q| !q.is_empty());
        let cat_lower = category.map(|c| c.trim().to_lowercase()).filter(|c| !c.is_empty() && c != "all");
        let tag_lower = tag.map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty());
        let ent_lower = entity_type.map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty() && e != "all");

        let mut res = Vec::new();
        for r in rows {
            let vars: Vec<String> = r
                .try_get::<Option<String>, _>("variants_json")
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let tags: Vec<String> = r
                .try_get::<Option<String>, _>("tags_json")
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let canonical_name: String = r.get("canonical_name");
            let role: Option<String> = r.try_get("role").ok().flatten();
            let cat: Option<String> = r.try_get("category").ok().flatten();
            let ent: String = r.try_get::<Option<String>, _>("entity_type").ok().flatten().unwrap_or_else(|| "person".to_string());

            if let Some(ref q) = q_lower {
                let match_name = canonical_name.to_lowercase().contains(q);
                let match_role = role.as_deref().map(|s| s.to_lowercase().contains(q)).unwrap_or(false);
                let match_tag = tags.iter().any(|t| t.to_lowercase().contains(q));
                if !match_name && !match_role && !match_tag {
                    continue;
                }
            }
            if let Some(ref c) = cat_lower {
                if !cat.as_deref().map(|s| s.eq_ignore_ascii_case(c)).unwrap_or(false) {
                    continue;
                }
            }
            if let Some(ref t) = tag_lower {
                if !tags.iter().any(|item| item.to_lowercase().contains(t)) {
                    continue;
                }
            }
            if let Some(ref e) = ent_lower {
                if !ent.eq_ignore_ascii_case(e) {
                    continue;
                }
            }

            let mentions: i64 = r.try_get("total_mentions").unwrap_or(0);
            res.push(Person {
                id: r.get("id"),
                canonical_name,
                variants: vars,
                role,
                public_figure: r.get::<i64, _>("public_figure") != 0,
                wikidata_id: r.try_get("wikidata_id").ok().flatten(),
                summary: r.try_get("summary").ok().flatten(),
                researched_at: r.try_get("researched_at").ok().flatten(),
                mentions: Some(mentions),
                found_in: None,
                tags,
                rating: r.try_get::<i64, _>("rating").unwrap_or(50),
                category: cat,
                notes: r.try_get("notes").ok().flatten(),
                detailed_profile_json: r.try_get("detailed_profile_json").ok().flatten(),
                entity_type: ent,
            });
        }

        if let Some(sort) = sort_by {
            match sort {
                "name" => res.sort_by(|a, b| a.canonical_name.cmp(&b.canonical_name)),
                "mentions" => res.sort_by(|a, b| b.mentions.unwrap_or(0).cmp(&a.mentions.unwrap_or(0))),
                "researched" => res.sort_by(|a, b| b.researched_at.cmp(&a.researched_at)),
                _ => res.sort_by(|a, b| b.rating.cmp(&a.rating)),
            }
        }

        Ok(res)
    }

    pub async fn get_person_by_id(&self, person_id: i64) -> Result<Option<Person>> {
        let row_opt = sqlx::query("SELECT * FROM people WHERE id = ?1")
            .bind(person_id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(r) = row_opt else {
            return Ok(None);
        };
        let vars: Vec<String> = r
            .try_get::<Option<String>, _>("variants_json")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let tags: Vec<String> = r
            .try_get::<Option<String>, _>("tags_json")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Ok(Some(Person {
            id: r.get("id"),
            canonical_name: r.get("canonical_name"),
            variants: vars,
            role: r.try_get("role").ok().flatten(),
            public_figure: r.get::<i64, _>("public_figure") != 0,
            wikidata_id: r.try_get("wikidata_id").ok().flatten(),
            summary: r.try_get("summary").ok().flatten(),
            researched_at: r.try_get("researched_at").ok().flatten(),
            mentions: None,
            found_in: None,
            tags,
            rating: r.try_get::<i64, _>("rating").unwrap_or(50),
            category: r.try_get("category").ok().flatten(),
            notes: r.try_get("notes").ok().flatten(),
            detailed_profile_json: r.try_get("detailed_profile_json").ok().flatten(),
            entity_type: r.try_get::<Option<String>, _>("entity_type").ok().flatten().unwrap_or_else(|| "person".to_string()),
        }))
    }

    pub async fn create_person(
        &self,
        canonical_name: &str,
        role: Option<&str>,
        category: Option<&str>,
        tags: &[String],
        rating: i64,
        notes: Option<&str>,
        public_figure: bool,
        entity_type: Option<&str>,
    ) -> Result<i64> {
        let tags_json = serde_json::to_string(tags)?;
        let vars_json = serde_json::to_string(&[canonical_name])?;
        let ent = entity_type.unwrap_or("person");
        let res = sqlx::query(
            "INSERT INTO people (canonical_name, variants_json, role, category, tags_json, rating, notes, public_figure, entity_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(canonical_name)
        .bind(&vars_json)
        .bind(role)
        .bind(category)
        .bind(&tags_json)
        .bind(rating)
        .bind(notes)
        .bind(if public_figure { 1i64 } else { 0i64 })
        .bind(ent)
        .execute(&self.pool)
        .await?;
        Ok(res.last_insert_rowid())
    }

    pub async fn update_person_full(
        &self,
        person_id: i64,
        canonical_name: &str,
        role: Option<&str>,
        category: Option<&str>,
        tags: &[String],
        rating: i64,
        notes: Option<&str>,
        public_figure: bool,
        summary: Option<&str>,
        detailed_profile_json: Option<&str>,
        entity_type: Option<&str>,
    ) -> Result<()> {
        let tags_json = serde_json::to_string(tags)?;
        sqlx::query(
            "UPDATE people SET
               canonical_name = ?2,
               role = ?3,
               category = ?4,
               tags_json = ?5,
               rating = ?6,
               notes = ?7,
               public_figure = ?8,
               summary = COALESCE(?9, summary),
               detailed_profile_json = COALESCE(?10, detailed_profile_json),
               entity_type = COALESCE(?11, entity_type)
             WHERE id = ?1",
        )
        .bind(person_id)
        .bind(canonical_name)
        .bind(role)
        .bind(category)
        .bind(&tags_json)
        .bind(rating)
        .bind(notes)
        .bind(if public_figure { 1i64 } else { 0i64 })
        .bind(summary)
        .bind(detailed_profile_json)
        .bind(entity_type)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_person_research_data(
        &self,
        person_id: i64,
        role: Option<&str>,
        summary: Option<&str>,
        tags: &[String],
        detailed_profile_json: Option<&str>,
        public_figure: bool,
        category: Option<&str>,
        entity_type: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(tags)?;
        sqlx::query(
            "UPDATE people SET
               role = COALESCE(?2, role),
               summary = COALESCE(?3, summary),
               tags_json = CASE WHEN ?4 != '[]' THEN ?4 ELSE tags_json END,
               detailed_profile_json = COALESCE(?5, detailed_profile_json),
               public_figure = CASE WHEN ?6 = 1 THEN 1 ELSE public_figure END,
               researched_at = ?7,
               category = COALESCE(?8, category),
               entity_type = COALESCE(?9, entity_type)
             WHERE id = ?1",
        )
        .bind(person_id)
        .bind(role)
        .bind(summary)
        .bind(&tags_json)
        .bind(detailed_profile_json)
        .bind(if public_figure { 1i64 } else { 0i64 })
        .bind(&now)
        .bind(category)
        .bind(entity_type)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_person(&self, person_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM video_people WHERE person_id = ?1")
            .bind(person_id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM people WHERE id = ?1")
            .bind(person_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_person_classification(
        &self,
        person_id: i64,
        entity_type: &str,
        category: Option<&str>,
        append_tag: Option<&str>,
    ) -> Result<()> {
        let row_opt = sqlx::query("SELECT tags_json, category FROM people WHERE id = ?1")
            .bind(person_id)
            .fetch_optional(&self.pool)
            .await?;
        if let Some(row) = row_opt {
            let mut tags: Vec<String> = row
                .try_get::<Option<String>, _>("tags_json")
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let current_cat: Option<String> = row.try_get("category").ok().flatten();
            if let Some(tag) = append_tag {
                if !tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                    tags.push(tag.to_string());
                }
            }
            let final_cat = category.or(current_cat.as_deref());
            let tags_json = serde_json::to_string(&tags)?;
            sqlx::query(
                "UPDATE people SET entity_type = ?2, category = ?3, tags_json = ?4 WHERE id = ?1",
            )
            .bind(person_id)
            .bind(entity_type)
            .bind(final_cat)
            .bind(&tags_json)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn reassign_video_person(&self, video_id: &str, old_person_id: i64, new_person_id: i64) -> Result<()> {
        let row_opt = sqlx::query("SELECT mentions, found_in FROM video_people WHERE video_id = ?1 AND person_id = ?2")
            .bind(video_id)
            .bind(old_person_id)
            .fetch_optional(&self.pool)
            .await?;
        let (mentions, found_in) = match row_opt {
            Some(r) => (r.try_get::<i64, _>("mentions").unwrap_or(1), r.try_get::<Option<String>, _>("found_in").ok().flatten().unwrap_or_else(|| "manual_correction".to_string())),
            None => (1i64, "manual_correction".to_string()),
        };
        sqlx::query("DELETE FROM video_people WHERE video_id = ?1 AND person_id = ?2")
            .bind(video_id)
            .bind(old_person_id)
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "INSERT INTO video_people (video_id, person_id, mentions, found_in) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(video_id, person_id) DO UPDATE SET mentions = excluded.mentions, found_in = excluded.found_in",
        )
        .bind(video_id)
        .bind(new_person_id)
        .bind(mentions)
        .bind(found_in)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_person_from_video(&self, video_id: &str, person_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM video_people WHERE video_id = ?1 AND person_id = ?2")
            .bind(video_id)
            .bind(person_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_person_to_video(&self, video_id: &str, person_id: i64, mentions: i64, found_in: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO video_people (video_id, person_id, mentions, found_in) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(video_id, person_id) DO UPDATE SET mentions = excluded.mentions, found_in = excluded.found_in",
        )
        .bind(video_id)
        .bind(person_id)
        .bind(mentions)
        .bind(found_in)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn merge_people(&self, source_id: i64, target_id: i64) -> Result<()> {
        self.merge_people_with_name(source_id, target_id, None).await
    }

    pub async fn merge_people_with_name(
        &self,
        source_id: i64,
        target_id: i64,
        custom_name: Option<&str>,
    ) -> Result<()> {
        if source_id == target_id {
            return Ok(());
        }

        let source_row = sqlx::query("SELECT canonical_name, variants_json, tags_json FROM people WHERE id = ?1")
            .bind(source_id)
            .fetch_optional(&self.pool)
            .await?;

        let target_row = sqlx::query("SELECT canonical_name, variants_json, tags_json FROM people WHERE id = ?1")
            .bind(target_id)
            .fetch_optional(&self.pool)
            .await?;

        if let (Some(s_row), Some(t_row)) = (source_row, target_row) {
            let s_name: String = s_row.get("canonical_name");
            let mut t_variants: Vec<String> = t_row
                .try_get::<Option<String>, _>("variants_json")
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let s_variants: Vec<String> = s_row
                .try_get::<Option<String>, _>("variants_json")
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            if !t_variants.iter().any(|v| v.eq_ignore_ascii_case(&s_name)) {
                t_variants.push(s_name);
            }
            for v in s_variants {
                if !t_variants.iter().any(|x| x.eq_ignore_ascii_case(&v)) {
                    t_variants.push(v);
                }
            }

            let mut t_tags: Vec<String> = t_row
                .try_get::<Option<String>, _>("tags_json")
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let s_tags: Vec<String> = s_row
                .try_get::<Option<String>, _>("tags_json")
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            for tag in s_tags {
                if !t_tags.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
                    t_tags.push(tag);
                }
            }

            let new_vars_json = serde_json::to_string(&t_variants)?;
            let new_tags_json = serde_json::to_string(&t_tags)?;

            if let Some(cn) = custom_name {
                let trimmed = cn.trim();
                if !trimmed.is_empty() {
                    sqlx::query("UPDATE people SET canonical_name = ?2, variants_json = ?3, tags_json = ?4 WHERE id = ?1")
                        .bind(target_id)
                        .bind(trimmed)
                        .bind(new_vars_json)
                        .bind(new_tags_json)
                        .execute(&self.pool)
                        .await?;
                } else {
                    sqlx::query("UPDATE people SET variants_json = ?2, tags_json = ?3 WHERE id = ?1")
                        .bind(target_id)
                        .bind(new_vars_json)
                        .bind(new_tags_json)
                        .execute(&self.pool)
                        .await?;
                }
            } else {
                sqlx::query("UPDATE people SET variants_json = ?2, tags_json = ?3 WHERE id = ?1")
                    .bind(target_id)
                    .bind(new_vars_json)
                    .bind(new_tags_json)
                    .execute(&self.pool)
                    .await?;
            }
        }

        let links = sqlx::query("SELECT video_id, mentions, found_in FROM video_people WHERE person_id = ?1")
            .bind(source_id)
            .fetch_all(&self.pool)
            .await?;

        for r in links {
            let vid: String = r.get("video_id");
            let m: i64 = r.try_get("mentions").unwrap_or(1);
            let fi: String = r.try_get("found_in").unwrap_or_else(|_| "merge".to_string());
            self.add_person_to_video(&vid, target_id, m, &fi).await?;
        }

        sqlx::query("DELETE FROM video_people WHERE person_id = ?1")
            .bind(source_id)
            .execute(&self.pool)
            .await?;

        let _ = sqlx::query("UPDATE entity_intel_events SET person_id = ?2 WHERE person_id = ?1")
            .bind(source_id)
            .bind(target_id)
            .execute(&self.pool)
            .await;

        let _ = sqlx::query("DELETE FROM entity_sanctions_cache WHERE person_id = ?1")
            .bind(source_id)
            .execute(&self.pool)
            .await;

        sqlx::query("DELETE FROM people WHERE id = ?1")
            .bind(source_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn save_claim_feedback(
        &self,
        claim_id: i64,
        video_id: &str,
        rating: &str,
        suggested_verdict: Option<&str>,
        notes: Option<&str>,
    ) -> Result<()> {
        sqlx::query("INSERT INTO claim_verification_feedback (claim_id, video_id, rating, suggested_verdict, notes, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
            .bind(claim_id)
            .bind(video_id)
            .bind(rating)
            .bind(suggested_verdict)
            .bind(notes)
            .bind(Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn latest_claim_notes(&self, claim_id: i64) -> Result<Option<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT notes FROM claim_verification_feedback WHERE claim_id = ?1 AND rating = 'edited' AND notes IS NOT NULL AND TRIM(notes) <> '' ORDER BY id DESC LIMIT 5",
        )
        .bind(claim_id)
        .fetch_all(&self.pool)
        .await?;
        if rows.is_empty() {
            return Ok(None);
        }
        let mut notes: Vec<String> = rows.into_iter().map(|r| r.0).collect();
        notes.reverse();
        Ok(Some(notes.join("\n")))
    }

    pub async fn split_compound_person(&self, source_id: i64, target_names: &[String]) -> Result<Vec<i64>> {
        let mut target_ids = Vec::new();
        let now = Utc::now().to_rfc3339();

        for name in target_names {
            let clean = name.trim();
            if clean.is_empty() {
                continue;
            }
            let row_opt = sqlx::query("SELECT id FROM people WHERE LOWER(canonical_name) = LOWER(?1) LIMIT 1")
                .bind(clean)
                .fetch_optional(&self.pool)
                .await?;

            let tid = if let Some(r) = row_opt {
                r.get::<i64, _>("id")
            } else {
                let vars_json = serde_json::to_string(&vec![clean.to_string()])?;
                let res = sqlx::query(
                    "INSERT INTO people (canonical_name, variants_json, public_figure, entity_type, researched_at) VALUES (?1, ?2, 1, 'person', ?3)",
                )
                .bind(clean)
                .bind(vars_json)
                .bind(&now)
                .execute(&self.pool)
                .await?;
                res.last_insert_rowid()
            };
            target_ids.push(tid);
        }

        let links = sqlx::query("SELECT video_id, mentions, found_in FROM video_people WHERE person_id = ?1")
            .bind(source_id)
            .fetch_all(&self.pool)
            .await?;

        for r in links {
            let vid: String = r.get("video_id");
            let m: i64 = r.try_get("mentions").unwrap_or(1);
            let fi: String = r.try_get("found_in").unwrap_or_else(|_| "split".to_string());
            for tid in &target_ids {
                self.add_person_to_video(&vid, *tid, m, &fi).await?;
            }
        }

        sqlx::query("DELETE FROM video_people WHERE person_id = ?1")
            .bind(source_id)
            .execute(&self.pool)
            .await?;

        let _ = sqlx::query("DELETE FROM entity_intel_events WHERE person_id = ?1")
            .bind(source_id)
            .execute(&self.pool)
            .await;

        let _ = sqlx::query("DELETE FROM entity_sanctions_cache WHERE person_id = ?1")
            .bind(source_id)
            .execute(&self.pool)
            .await;

        sqlx::query("DELETE FROM people WHERE id = ?1")
            .bind(source_id)
            .execute(&self.pool)
            .await?;

        Ok(target_ids)
    }

    pub async fn cleanup_person_as_tag(&self, source_id: i64, target_name: &str, tag: &str) -> Result<i64> {
        let clean_target = target_name.trim();
        let now = Utc::now().to_rfc3339();

        let row_opt = sqlx::query("SELECT id, tags_json FROM people WHERE LOWER(canonical_name) = LOWER(?1) LIMIT 1")
            .bind(clean_target)
            .fetch_optional(&self.pool)
            .await?;

        let (target_id, mut tags) = if let Some(r) = row_opt {
            let tid: i64 = r.get("id");
            let t_vec: Vec<String> = r
                .try_get::<Option<String>, _>("tags_json")
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            (tid, t_vec)
        } else {
            let vars_json = serde_json::to_string(&vec![clean_target.to_string()])?;
            let res = sqlx::query(
                "INSERT INTO people (canonical_name, variants_json, public_figure, entity_type, researched_at) VALUES (?1, ?2, 1, 'person', ?3)",
            )
            .bind(clean_target)
            .bind(vars_json)
            .bind(&now)
            .execute(&self.pool)
            .await?;
            (res.last_insert_rowid(), Vec::new())
        };

        if !tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
            tags.push(tag.to_string());
            let tags_json = serde_json::to_string(&tags)?;
            sqlx::query("UPDATE people SET tags_json = ?2 WHERE id = ?1")
                .bind(target_id)
                .bind(tags_json)
                .execute(&self.pool)
                .await?;
        }

        let links = sqlx::query("SELECT video_id, mentions, found_in FROM video_people WHERE person_id = ?1")
            .bind(source_id)
            .fetch_all(&self.pool)
            .await?;

        for r in links {
            let vid: String = r.get("video_id");
            let m: i64 = r.try_get("mentions").unwrap_or(1);
            let fi: String = r.try_get("found_in").unwrap_or_else(|_| "tag_cleanup".to_string());
            self.add_person_to_video(&vid, target_id, m, &fi).await?;
        }

        sqlx::query("DELETE FROM video_people WHERE person_id = ?1")
            .bind(source_id)
            .execute(&self.pool)
            .await?;

        let _ = sqlx::query("DELETE FROM entity_intel_events WHERE person_id = ?1")
            .bind(source_id)
            .execute(&self.pool)
            .await;

        let _ = sqlx::query("DELETE FROM entity_sanctions_cache WHERE person_id = ?1")
            .bind(source_id)
            .execute(&self.pool)
            .await;

        sqlx::query("DELETE FROM people WHERE id = ?1")
            .bind(source_id)
            .execute(&self.pool)
            .await?;

        Ok(target_id)
    }

    pub async fn record_intel_event(
        &self,
        person_id: i64,
        source_name: &str,
        title: &str,
        url: &str,
        snippet: &str,
    ) -> Result<i64> {
        let existing = sqlx::query("SELECT id FROM entity_intel_events WHERE person_id = ?1 AND url = ?2 LIMIT 1")
            .bind(person_id)
            .bind(url)
            .fetch_optional(&self.pool)
            .await?;
        if let Some(r) = existing {
            return Ok(r.get::<i64, _>("id"));
        }
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "INSERT INTO entity_intel_events (person_id, source_name, title, url, discovered_at, snippet)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(person_id)
        .bind(source_name)
        .bind(title)
        .bind(url)
        .bind(&now)
        .bind(snippet)
        .execute(&self.pool)
        .await?;
        Ok(res.last_insert_rowid())
    }

    pub async fn get_intel_events_for_person(
        &self,
        person_id: i64,
        limit: i64,
    ) -> Result<Vec<EntityIntelEvent>> {
        let rows = sqlx::query(
            "SELECT id, person_id, source_name, title, url, discovered_at, snippet
             FROM entity_intel_events
             WHERE person_id = ?1
             ORDER BY discovered_at DESC, id DESC
             LIMIT ?2",
        )
        .bind(person_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let mut list = Vec::new();
        for r in rows {
            list.push(EntityIntelEvent {
                id: r.get("id"),
                person_id: r.get("person_id"),
                source_name: r.get("source_name"),
                title: r.get("title"),
                url: r.get("url"),
                discovered_at: r.get("discovered_at"),
                snippet: r.get("snippet"),
            });
        }
        Ok(list)
    }

    pub async fn save_sanctions_matches(
        &self,
        person_id: i64,
        matches: &[SanctionsMatch],
    ) -> Result<()> {
        let raw = serde_json::to_string(matches)?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO entity_sanctions_cache (person_id, payload_json, checked_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(person_id) DO UPDATE SET
               payload_json = excluded.payload_json,
               checked_at = excluded.checked_at",
        )
        .bind(person_id)
        .bind(&raw)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_sanctions_matches(
        &self,
        person_id: i64,
    ) -> Result<Option<(Vec<SanctionsMatch>, String)>> {
        let row_opt = sqlx::query("SELECT payload_json, checked_at FROM entity_sanctions_cache WHERE person_id = ?1")
            .bind(person_id)
            .fetch_optional(&self.pool)
            .await?;
        if let Some(r) = row_opt {
            let json_str: String = r.get("payload_json");
            let checked_at: String = r.get("checked_at");
            let list: Vec<SanctionsMatch> = serde_json::from_str(&json_str).unwrap_or_default();
            Ok(Some((list, checked_at)))
        } else {
            Ok(None)
        }
    }

    pub async fn touch_person_monitored(&self, person_id: i64) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE people SET last_monitored_at = ?2 WHERE id = ?1")
            .bind(person_id)
            .bind(&now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_people_due_for_monitoring(&self, limit: i64) -> Result<Vec<Person>> {
        let rows = sqlx::query(
            "SELECT * FROM people
             WHERE public_figure = 1 OR researched_at IS NOT NULL
             ORDER BY last_monitored_at ASC NULLS FIRST
             LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let vars: Vec<String> = r
                    .try_get::<Option<String>, _>("variants_json")
                    .ok()
                    .flatten()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                let tags: Vec<String> = r
                    .try_get::<Option<String>, _>("tags_json")
                    .ok()
                    .flatten()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                Person {
                    id: r.get("id"),
                    canonical_name: r.get("canonical_name"),
                    variants: vars,
                    role: r.try_get("role").ok().flatten(),
                    public_figure: r.get::<i64, _>("public_figure") != 0,
                    wikidata_id: r.try_get("wikidata_id").ok().flatten(),
                    summary: r.try_get("summary").ok().flatten(),
                    researched_at: r.try_get("researched_at").ok().flatten(),
                    mentions: None,
                    found_in: None,
                    tags,
                    rating: r.try_get::<i64, _>("rating").unwrap_or(50),
                    category: r.try_get("category").ok().flatten(),
                    notes: r.try_get("notes").ok().flatten(),
                    detailed_profile_json: r.try_get("detailed_profile_json").ok().flatten(),
                    entity_type: r.try_get::<Option<String>, _>("entity_type").ok().flatten().unwrap_or_else(|| "person".to_string()),
                }
            })
            .collect())
    }

    pub async fn get_decisions_for_video(&self, video_id: &str) -> Result<Vec<Decision>> {
        let rows = sqlx::query("SELECT * FROM decisions WHERE video_id = ?1 ORDER BY id DESC")
            .bind(video_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| Decision {
                id: r.get("id"),
                video_id: r.get("video_id"),
                action: r.get("action"),
                note: r.try_get("note").ok().flatten(),
                decided_at: r.get("decided_at"),
            })
            .collect())
    }

    pub async fn list_seeds(&self, active_only: bool) -> Result<Vec<Seed>> {
        let rows = if active_only {
            sqlx::query("SELECT * FROM seeds WHERE active = 1 ORDER BY kind ASC, value ASC")
                .fetch_all(&self.pool)
                .await?
        } else {
            sqlx::query("SELECT * FROM seeds ORDER BY kind ASC, value ASC")
                .fetch_all(&self.pool)
                .await?
        };
        Ok(rows
            .into_iter()
            .map(|r| Seed {
                id: r.get("id"),
                kind: r.get("kind"),
                value: r.get("value"),
                topic: r.try_get("topic").ok().flatten(),
                active: r.get::<i64, _>("active") != 0,
                last_run_at: r.try_get("last_run_at").ok().flatten(),
            })
            .collect())
    }

    pub async fn upsert_seed(&self, kind: &str, value: &str, topic: Option<&str>, active: bool) -> Result<Seed> {
        let act = if active { 1i64 } else { 0i64 };
        sqlx::query(
            "INSERT INTO seeds (kind, value, topic, active)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(kind, value) DO UPDATE SET
               topic = COALESCE(excluded.topic, seeds.topic),
               active = excluded.active",
        )
        .bind(kind)
        .bind(value)
        .bind(topic)
        .bind(act)
        .execute(&self.pool)
        .await?;

        let row = sqlx::query("SELECT * FROM seeds WHERE kind = ?1 AND value = ?2")
            .bind(kind)
            .bind(value)
            .fetch_one(&self.pool)
            .await?;

        Ok(Seed {
            id: row.get("id"),
            kind: row.get("kind"),
            value: row.get("value"),
            topic: row.try_get("topic").ok().flatten(),
            active: row.get::<i64, _>("active") != 0,
            last_run_at: row.try_get("last_run_at").ok().flatten(),
        })
    }

    pub async fn delete_seed(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM seeds WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn mark_seed_ran(&self, id: i64) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE seeds SET last_run_at = ?2 WHERE id = ?1")
            .bind(id)
            .bind(&now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn enqueue_job(&self, video_id: Option<&str>, stage: Stage) -> Result<Job> {
        if let Some(vid) = video_id {
            let existing = sqlx::query(
                "SELECT * FROM jobs
                 WHERE video_id = ?1 AND stage = ?2 AND status IN ('queued', 'running')
                 ORDER BY id DESC LIMIT 1",
            )
            .bind(vid)
            .bind(stage.as_str())
            .fetch_optional(&self.pool)
            .await?;

            if let Some(r) = existing {
                return Ok(Self::map_job_row(&r));
            }
        }

        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "INSERT INTO jobs (video_id, stage, status, attempts, updated_at)
             VALUES (?1, ?2, 'queued', 0, ?3)",
        )
        .bind(video_id)
        .bind(stage.as_str())
        .bind(&now)
        .execute(&self.pool)
        .await?;

        let id = res.last_insert_rowid();
        let row = sqlx::query("SELECT * FROM jobs WHERE id = ?1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        Ok(Self::map_job_row(&row))
    }

    fn map_job_row(row: &SqliteRow) -> Job {
        Job {
            id: row.get("id"),
            video_id: row.try_get("video_id").ok().flatten(),
            stage: row.get("stage"),
            status: row.get("status"),
            attempts: row.get("attempts"),
            last_error: row.try_get("last_error").ok().flatten(),
            run_after: row.try_get("run_after").ok().flatten(),
            updated_at: row.get("updated_at"),
        }
    }

    pub async fn get_job(&self, job_id: i64) -> Result<Option<Job>> {
        let row = sqlx::query("SELECT * FROM jobs WHERE id = ?1")
            .bind(job_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(Self::map_job_row))
    }

    pub async fn claim_next_job(&self, stage_filter: Option<Stage>) -> Result<Option<Job>> {
        let now = Utc::now().to_rfc3339();
        let mut tx = self.pool.begin().await?;

        let candidate = if let Some(st) = stage_filter {
            sqlx::query(
                "SELECT j.* FROM jobs j
                 LEFT JOIN videos v ON v.id = j.video_id
                 WHERE j.status IN ('queued', 'deferred')
                   AND j.stage = ?1
                   AND (j.run_after IS NULL OR j.run_after <= ?2)
                 ORDER BY CASE WHEN v.discovered_via LIKE 'paste%' OR v.lane = 'pasted' THEN 0 ELSE 1 END, j.id ASC
                 LIMIT 1",
            )
            .bind(st.as_str())
            .bind(&now)
            .fetch_optional(&mut *tx)
            .await?
        } else {
            sqlx::query(
                "SELECT j.* FROM jobs j
                 LEFT JOIN videos v ON v.id = j.video_id
                 WHERE j.status IN ('queued', 'deferred')
                   AND (j.run_after IS NULL OR j.run_after <= ?1)
                 ORDER BY CASE WHEN v.discovered_via LIKE 'paste%' OR v.lane = 'pasted' THEN 0 ELSE 1 END, j.id ASC
                 LIMIT 1",
            )
            .bind(&now)
            .fetch_optional(&mut *tx)
            .await?
        };

        let Some(row) = candidate else {
            tx.commit().await?;
            return Ok(None);
        };

        let job_id: i64 = row.get("id");
        let updated = sqlx::query(
            "UPDATE jobs
             SET status = 'running', attempts = attempts + 1, updated_at = ?2
             WHERE id = ?1 AND status IN ('queued', 'deferred')",
        )
        .bind(job_id)
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        if updated.rows_affected() == 0 {
            tx.commit().await?;
            return Ok(None);
        }

        let claimed = sqlx::query("SELECT * FROM jobs WHERE id = ?1")
            .bind(job_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;

        Ok(Some(Self::map_job_row(&claimed)))
    }

    pub async fn complete_job(&self, job_id: i64) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE jobs SET status = 'succeeded', last_error = NULL, run_after = NULL, updated_at = ?2 WHERE id = ?1",
        )
        .bind(job_id)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_job_running(&self, job_id: i64) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE jobs SET status = 'running', updated_at = ?2 WHERE id = ?1",
        )
        .bind(job_id)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn defer_job_until(&self, job_id: i64, run_after: DateTime<Utc>, reason: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let after = run_after.to_rfc3339();
        sqlx::query(
            "UPDATE jobs
             SET status = 'deferred',
                 attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END,
                 last_error = ?2,
                 run_after = ?3,
                 updated_at = ?4
             WHERE id = ?1",
        )
        .bind(job_id)
        .bind(reason)
        .bind(&after)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn fail_or_retry_job(&self, job: &Job, error_msg: &str, max_attempts: i64, retryable: bool) -> Result<bool> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        if retryable && job.attempts < max_attempts {
            let delay = compute_backoff_delay(job.attempts.max(1) as u32);
            let run_after = (now + chrono::Duration::from_std(delay).unwrap_or_else(|_| chrono::Duration::seconds(5))).to_rfc3339();
            sqlx::query(
                "UPDATE jobs SET status = 'queued', last_error = ?2, run_after = ?3, updated_at = ?4 WHERE id = ?1",
            )
            .bind(job.id)
            .bind(error_msg)
            .bind(&run_after)
            .bind(&now_str)
            .execute(&self.pool)
            .await?;
            return Ok(true);
        }

        sqlx::query(
            "UPDATE jobs SET status = 'failed', last_error = ?2, run_after = NULL, updated_at = ?3 WHERE id = ?1",
        )
        .bind(job.id)
        .bind(error_msg)
        .bind(&now_str)
        .execute(&self.pool)
        .await?;

        if let Some(vid) = &job.video_id {
            sqlx::query("UPDATE videos SET state = 'FAILED', state_reason = ?2, lane = 'failed' WHERE id = ?1")
                .bind(vid)
                .bind(error_msg)
                .execute(&self.pool)
                .await?;
        }
        Ok(false)
    }

    pub async fn recover_running_jobs(&self) -> Result<u64> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE jobs
             SET status = 'queued',
                 attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END,
                 run_after = NULL,
                 updated_at = ?1
             WHERE status = 'running'",
        )
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    pub async fn recover_stalled_jobs(&self, max_running_seconds: i64) -> Result<u64> {
        let now = Utc::now();
        let cutoff = (now - chrono::Duration::seconds(max_running_seconds)).to_rfc3339();
        let now_str = now.to_rfc3339();
        let res = sqlx::query(
            "UPDATE jobs
             SET status = 'queued',
                 attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END,
                 run_after = NULL,
                 updated_at = ?1
             WHERE status = 'running' AND updated_at < ?2",
        )
        .bind(&now_str)
        .bind(&cutoff)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    pub async fn defer_job_to(&self, job_id: i64, run_after: &str, reason: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE jobs
             SET status = 'deferred',
                 run_after = ?2,
                 last_error = ?3,
                 updated_at = ?4
             WHERE id = ?1",
        )
        .bind(job_id)
        .bind(run_after)
        .bind(reason)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn queue_counts(&self) -> Result<serde_json::Value> {
        let rows = sqlx::query("SELECT status, COUNT(*) AS cnt FROM jobs GROUP BY status")
            .fetch_all(&self.pool)
            .await?;
        let mut map = serde_json::Map::new();
        for r in rows {
            let st: String = r.get("status");
            let cnt: i64 = r.get("cnt");
            map.insert(st, serde_json::Value::from(cnt));
        }
        Ok(serde_json::Value::Object(map))
    }

    pub async fn record_provider_usage(
        &self,
        provider: &str,
        model: &str,
        tokens_in: u64,
        tokens_out: u64,
        is_429: bool,
    ) -> Result<()> {
        let day = Utc::now().format("%Y-%m-%d").to_string();
        let err_inc = if is_429 { 1i64 } else { 0i64 };
        sqlx::query(
            "INSERT INTO provider_usage (provider, model, day, requests, tokens_in, tokens_out, errors_429)
             VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6)
             ON CONFLICT(provider, model, day) DO UPDATE SET
               requests = provider_usage.requests + 1,
               tokens_in = provider_usage.tokens_in + excluded.tokens_in,
               tokens_out = provider_usage.tokens_out + excluded.tokens_out,
               errors_429 = provider_usage.errors_429 + excluded.errors_429",
        )
        .bind(provider)
        .bind(model)
        .bind(&day)
        .bind(tokens_in as i64)
        .bind(tokens_out as i64)
        .bind(err_inc)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn total_requests_today_for_prefix(&self, provider_prefix: &str) -> Result<u64> {
        let day = Utc::now().format("%Y-%m-%d").to_string();
        let row = sqlx::query(
            "SELECT COALESCE(SUM(requests), 0) AS total
             FROM provider_usage
             WHERE day = ?1 AND provider LIKE ?2 || '%'",
        )
        .bind(&day)
        .bind(provider_prefix)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get::<i64, _>("total").max(0) as u64)
    }

    pub async fn get_today_provider_usage(&self) -> Result<Vec<ProviderUsageSummary>> {
        let day = Utc::now().format("%Y-%m-%d").to_string();
        let rows = sqlx::query(
            "SELECT provider, model, day, requests, tokens_in, tokens_out, errors_429
             FROM provider_usage
             WHERE day = ?1",
        )
        .bind(&day)
        .fetch_all(&self.pool)
        .await?;

        let mut list = Vec::new();
        for r in rows {
            list.push(ProviderUsageSummary {
                provider: r.get("provider"),
                model: r.get("model"),
                day: r.get("day"),
                requests: r.get("requests"),
                tokens_in: r.get("tokens_in"),
                tokens_out: r.get("tokens_out"),
                errors_429: r.get("errors_429"),
            });
        }
        Ok(list)
    }

    pub async fn save_profile(&self, profile: &HardwareProfile) -> Result<()> {
        let hw = serde_json::to_string(&profile.hardware_json)?;
        let ch = serde_json::to_string(&profile.chosen_json)?;
        sqlx::query(
            "INSERT INTO profile (id, hardware_json, chosen_json, capacity_per_day, measured_at)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               hardware_json = excluded.hardware_json,
               chosen_json = excluded.chosen_json,
               capacity_per_day = excluded.capacity_per_day,
               measured_at = excluded.measured_at",
        )
        .bind(&hw)
        .bind(&ch)
        .bind(profile.capacity_per_day)
        .bind(&profile.measured_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_profile(&self) -> Result<Option<HardwareProfile>> {
        let row = sqlx::query("SELECT * FROM profile WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| {
            let hw_raw: String = r.get("hardware_json");
            let ch_raw: String = r.get("chosen_json");
            HardwareProfile {
                hardware_json: serde_json::from_str(&hw_raw).unwrap_or(serde_json::Value::Null),
                chosen_json: serde_json::from_str(&ch_raw).unwrap_or(serde_json::Value::Null),
                capacity_per_day: r.try_get("capacity_per_day").ok().flatten(),
                measured_at: r.get("measured_at"),
            }
        }))
    }

    pub async fn save_profile_for_power(
        &self,
        profile: &HardwareProfile,
        power_source: &str,
    ) -> Result<()> {
        let key = if power_source.eq_ignore_ascii_case("battery") {
            "profile_battery"
        } else {
            "profile_ac"
        };
        self.set_setting(key, &serde_json::to_value(profile)?).await?;
        self.save_profile(profile).await?;
        Ok(())
    }

    pub async fn get_profile_for_power(
        &self,
        power_source: &str,
    ) -> Result<Option<HardwareProfile>> {
        let key = if power_source.eq_ignore_ascii_case("battery") {
            "profile_battery"
        } else {
            "profile_ac"
        };
        if let Some(val) = self.get_setting(key).await? {
            if let Ok(prof) = serde_json::from_value::<HardwareProfile>(val) {
                return Ok(Some(prof));
            }
        }
        self.get_profile().await
    }

    pub async fn get_power_profiles(
        &self,
    ) -> Result<(Option<HardwareProfile>, Option<HardwareProfile>)> {
        let mut ac = self
            .get_setting("profile_ac")
            .await?
            .and_then(|v| serde_json::from_value::<HardwareProfile>(v).ok());
        let batt = self
            .get_setting("profile_battery")
            .await?
            .and_then(|v| serde_json::from_value::<HardwareProfile>(v).ok());
        if ac.is_none() {
            ac = self.get_profile().await.ok().flatten();
        }
        Ok((ac, batt))
    }

    pub async fn set_setting(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        let raw = serde_json::to_string(value)?;
        sqlx::query(
            "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
        )
        .bind(key)
        .bind(&raw)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_setting(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let row = sqlx::query("SELECT value_json FROM settings WHERE key = ?1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|r| {
            let s: String = r.get("value_json");
            serde_json::from_str(&s).ok()
        }))
    }

    pub async fn list_settings(&self) -> Result<serde_json::Map<String, serde_json::Value>> {
        let rows = sqlx::query("SELECT key, value_json FROM settings ORDER BY key ASC")
            .fetch_all(&self.pool)
            .await?;
        let mut map = serde_json::Map::new();
        for r in rows {
            let k: String = r.get("key");
            let v_raw: String = r.get("value_json");
            if let Ok(val) = serde_json::from_str(&v_raw) {
                map.insert(k, val);
            }
        }
        Ok(map)
    }
}
