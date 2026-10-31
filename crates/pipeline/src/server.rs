use std::convert::Infallible;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::cors::CorsLayer;
use leadfinder_core::{CoreEvent, Search, Stage, VideoFilter};
use leadfinder_probe::{apply_calibration_overrides, CalibrationOverrides};
use crate::engine::{CustomScanConfig, PipelineContext};

#[derive(Debug, Deserialize)]
pub struct TokenQuery {
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    pub format: Option<String>,
    pub lane: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PasteRequest {
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct OpenBrowserRequest {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct ListPeopleQuery {
    pub query: Option<String>,
    pub tag: Option<String>,
    pub category: Option<String>,
    pub sort_by: Option<String>,
    pub entity_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePersonRequest {
    pub canonical_name: String,
    pub role: Option<String>,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub rating: Option<i64>,
    pub notes: Option<String>,
    pub public_figure: Option<bool>,
    pub entity_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePersonRequest {
    pub canonical_name: String,
    pub role: Option<String>,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub rating: Option<i64>,
    pub notes: Option<String>,
    pub public_figure: Option<bool>,
    pub summary: Option<String>,
    pub detailed_profile_json: Option<String>,
    pub entity_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReassignVideoPersonRequest {
    pub new_person_id: Option<i64>,
    pub canonical_name: Option<String>,
    pub role: Option<String>,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub entity_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportRequest {
    pub content: String,
    #[serde(default = "default_import_label")]
    pub label: String,
}

fn default_import_label() -> String {
    "api".to_string()
}

#[derive(Debug, Deserialize)]
pub struct DiscoverTriggerRequest {
    #[serde(default = "default_max_per_seed")]
    pub max_per_seed: usize,
}

fn default_max_per_seed() -> usize {
    10
}

#[derive(Debug, Deserialize)]
pub struct VideoListQuery {
    pub lane: Option<String>,
    pub state: Option<String>,
    pub verdict: Option<String>,
    pub label: Option<String>,
    pub creator_id: Option<String>,
    pub q: Option<String>,
    pub min_score: Option<f64>,
    pub min_views: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct DecisionRequest {
    pub action: String,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TranscriptEditRequest {
    #[serde(alias = "text")]
    pub edited_text: String,
    #[serde(default)]
    pub rerun_claims: bool,
}

#[derive(Debug, Deserialize)]
pub struct RetranscribeRequest {
    #[serde(default = "default_retranscribe_warm")]
    pub warm_start: bool,
    #[serde(default = "default_retranscribe_claims")]
    pub rerun_claims: bool,
}

fn default_retranscribe_warm() -> bool {
    true
}

fn default_retranscribe_claims() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct SeedUpsertRequest {
    pub kind: String,
    pub value: String,
    pub topic: Option<String>,
    #[serde(default = "default_true_flag")]
    pub active: bool,
}

fn default_true_flag() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct WatchlistRequest {
    pub handle: String,
    pub watchlist: bool,
}

#[derive(Debug, Deserialize, Default)]
pub struct CalibrateRequest {
    #[serde(default)]
    pub force_cpu: bool,
    pub power_source: Option<String>,
    #[serde(default)]
    pub force_rerun: bool,
}

#[derive(Debug, Serialize)]
pub struct CalibrateStatusResponse {
    pub is_running: bool,
    pub progress: leadfinder_probe::BenchmarkState,
    pub power: leadfinder_probe::PowerInfo,
    pub ac_profile: Option<leadfinder_core::HardwareProfile>,
    pub battery_profile: Option<leadfinder_core::HardwareProfile>,
    pub active_profile: Option<leadfinder_core::HardwareProfile>,
}

#[derive(Debug, Deserialize)]
pub struct SwitchPowerRequest {
    pub power_source: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiKeyInfo {
    pub index: usize,
    pub masked: String,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub region: String,
    pub automated_discovery_enabled: bool,
    pub discovery_paused: bool,
    pub breakers: Vec<leadfinder_core::HostBreakerStatus>,
    pub queue: serde_json::Value,
    pub capacity_per_day: Option<f64>,
    pub cloud_enabled: bool,
    pub lane_counts: std::collections::HashMap<String, i64>,
}

pub fn build_router(ctx: Arc<PipelineContext>) -> Router {
    let protected = Router::new()
        .route("/api/health", get(handle_health))
        .route("/api/events", get(handle_events))
        .route("/api/inbox/paste", post(handle_paste))
        .route("/api/inbox/import", post(handle_import))
        .route("/api/discover/run", post(handle_discover_run))
        .route("/api/discover/scan", post(handle_discover_scan))
        .route("/api/rank/run", post(handle_rank_run))
        .route("/api/rank/pull-more", post(handle_pull_more))
        .route("/api/rank/tune", post(handle_tune))
        .route("/api/videos", get(handle_list_videos))
        .route("/api/videos/:id", get(handle_video_detail).delete(handle_delete_video))
        .route("/api/videos/:id/deepfake", get(handle_get_deepfake).post(handle_trigger_deepfake))
        .route("/api/videos/:id/decision", post(handle_decision))
        .route("/api/videos/:id/transcript", patch(handle_edit_transcript))
        .route("/api/videos/:id/retranscribe", post(handle_retranscribe))
        .route("/api/videos/:id/ai-check", post(handle_ai_check_transcript))
        .route("/api/videos/:id/google-ai-fix", post(handle_google_ai_fix))
        .route("/api/videos/:id/google-audio-transcribe", post(handle_google_audio_transcribe))
        .route("/api/google-ai/query", post(handle_google_ai_query))
        .route("/api/evidence/preview", get(handle_evidence_preview))
        .route("/api/evidence/:id", delete(handle_delete_evidence))
        .route("/api/open-browser", post(handle_open_browser))
        .route("/api/people", get(handle_list_people).post(handle_create_person))
        .route("/api/people/auto-classify", post(handle_auto_classify_people))
        .route("/api/people/dedup/proposals", get(handle_get_dedup_proposals))
        .route("/api/people/dedup/apply", post(handle_apply_dedup_proposal))
        .route("/api/people/dedup/auto", post(handle_auto_dedup_people))
        .route("/api/people/merge", post(handle_direct_merge_people))
        .route("/api/claims/:id/feedback", post(handle_claim_feedback))
        .route("/api/claims/:id/redact", post(handle_claim_redact))
        .route("/api/notifications", get(handle_get_notifications))
        .route("/api/people/:id", get(handle_get_person).put(handle_update_person).delete(handle_delete_person))
        .route("/api/people/:id/dossier", get(handle_get_person_dossier))
        .route("/api/people/:id/intel-stream", get(handle_get_person_intel_stream))
        .route("/api/people/:id/intel-stream/refresh", post(handle_refresh_person_intel_stream))
        .route("/api/people/:id/sanctions", get(handle_get_person_sanctions))
        .route("/api/people/:id/wayback", get(handle_get_person_wayback))
        .route("/api/people/:id/wayback/refresh", post(handle_refresh_person_wayback))
        .route("/api/people/:id/research", post(handle_research_person))
        .route("/api/videos/:video_id/people/:person_id/reassign", post(handle_reassign_video_person))
        .route("/api/videos/:video_id/people/:person_id", delete(handle_remove_video_person))
        .route("/api/seeds", get(handle_list_seeds).post(handle_upsert_seed))
        .route("/api/seeds/:id", delete(handle_delete_seed))
        .route("/api/creators", get(handle_list_creators))
        .route("/api/creators/watchlist", post(handle_set_watchlist))
        .route("/api/creators/:handle/assess-risk", post(handle_assess_creator_risk))
        .route("/api/profile", get(handle_get_profile))
        .route("/api/calibrate", post(handle_calibrate))
        .route("/api/calibrate/status", get(handle_calibrate_status))
        .route("/api/calibrate/switch-power", post(handle_calibrate_switch_power))
        .route("/api/calibrate/overrides", post(handle_calibrate_overrides))
        .route("/api/settings", get(handle_list_settings).post(handle_update_settings))
        .route("/api/settings/gemini-status", get(handle_gemini_status))
        .route("/api/settings/gemini-key", post(handle_set_gemini_key).delete(handle_delete_gemini_key))
        .route("/api/settings/gemini-keys/rotate", post(handle_rotate_gemini_key))
        .route("/api/models", get(handle_list_models))
        .route("/api/models/catalog", get(handle_get_model_catalog))
        .route("/api/models/download", post(handle_download_model))
        .route("/api/models/download/status", get(handle_download_model_status))
        .route("/api/models/download/cancel", post(handle_download_model_cancel))
        .route("/api/models/:kind/:filename", delete(handle_delete_model))
        .route("/api/models/scan-folder", post(handle_scan_models_folder))
        .route("/api/models/external/scan", post(handle_scan_external_ai_servers))
        .route("/api/models/roles", get(handle_get_model_roles).post(handle_update_model_roles))
        .route("/api/models/test", post(handle_test_model))
        .route("/api/export", get(handle_export))
        .route("/api/scale/status", get(handle_scale_status))
        .route("/api/system/shutdown", post(handle_system_shutdown))
        .route("/api/system/shutdown-when-idle", post(handle_system_shutdown_when_idle))
        .route("/api/system/exit-status", get(handle_system_exit_status))
        .route_layer(middleware::from_fn_with_state(ctx.clone(), auth_middleware));

    tokio::spawn({
        let ctx = ctx.clone();
        async move {
            let mut last_source = leadfinder_probe::detect_power_info().power_source;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                let current = leadfinder_probe::detect_power_info();
                if current.power_source != last_source {
                    let old = last_source.clone();
                    last_source = current.power_source.clone();
                    let prof = ctx.db.get_profile_for_power(&current.power_source).await.ok().flatten();
                    let has_profile = prof.is_some();
                    if let Some(ref p) = prof {
                        let _ = ctx.db.save_profile(p).await;
                    }
                    let _ = ctx.events_tx.send(leadfinder_core::CoreEvent::PowerChanged(serde_json::json!({
                        "power_source": current.power_source,
                        "old_source": old,
                        "new_source": current.power_source,
                        "ac_online": current.ac_online,
                        "battery_percent": current.battery_percent,
                        "has_profile": has_profile,
                        "capacity_per_day": prof.and_then(|p| p.capacity_per_day),
                    })));
                }
            }
        }
    });

    Router::new()
        .route("/metrics", get(handle_metrics))
        .nest_service("/api/media", tower_http::services::ServeDir::new(ctx.media_root()))
        .merge(protected)
        .layer(CorsLayer::permissive())
        .with_state(ctx)
}

async fn auth_middleware(
    State(ctx): State<Arc<PipelineContext>>,
    Query(tq): Query<TokenQuery>,
    req: Request,
    next: Next,
) -> Response {
    let header_token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim());

    let provided = header_token.or_else(|| tq.token.as_deref());
    if provided != Some(ctx.api_token.as_str()) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response();
    }

    next.run(req).await
}

async fn handle_health(State(ctx): State<Arc<PipelineContext>>) -> impl IntoResponse {
    let cfg = ctx.config.read().await.clone();
    let breakers = ctx.guard.breaker_statuses().await;
    let discovery_paused = breakers.iter().any(|b| b.is_open);
    let queue = ctx.db.queue_counts().await.unwrap_or_else(|_| serde_json::json!({}));
    let capacity_per_day = ctx
        .db
        .get_profile()
        .await
        .ok()
        .flatten()
        .and_then(|p| p.capacity_per_day);
    let lane_counts = ctx.db.get_lane_counts().await.unwrap_or_default();

    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        region: cfg.discovery.region,
        automated_discovery_enabled: cfg.discovery.automated_discovery_enabled,
        discovery_paused,
        breakers,
        queue,
        capacity_per_day,
        cloud_enabled: cfg.providers.cloud_enabled,
        lane_counts,
    })
}

async fn handle_events(
    State(ctx): State<Arc<PipelineContext>>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let rx = ctx.events_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| async move {
        match msg {
            Ok(ev) => {
                let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".to_string());
                Some(Ok(Event::default().data(payload)))
            }
            Err(_) => None,
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10)).text("keepalive"))
}

async fn handle_paste(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<PasteRequest>,
) -> Response {
    match ctx.paste_urls(&body.text).await {
        Ok(videos) => {
            ctx.spawn_metadata_enrichment(&videos);
            Json(serde_json::json!({
                "ingested": videos.len(),
                "videos": videos
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_import(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<ImportRequest>,
) -> Response {
    match ctx.import_partner_content(&body.content, &body.label).await {
        Ok(videos) => Json(serde_json::json!({
            "ingested": videos.len(),
            "videos": videos
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct PullMoreRequest {
    pub count: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct TuneRequest {
    pub confirm: Option<bool>,
}

async fn handle_rank_run(State(ctx): State<Arc<PipelineContext>>) -> Response {
    match ctx.rank_all().await {
        Ok(summary) => Json(summary).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_pull_more(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<PullMoreRequest>,
) -> Response {
    let count = body.count.unwrap_or(10).clamp(1, 200);
    match ctx.pull_more(count).await {
        Ok(promoted) => Json(serde_json::json!({
            "promoted": promoted,
            "pulled_count": promoted
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_tune(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<TuneRequest>,
) -> Response {
    match ctx.tune_ranking(body.confirm.unwrap_or(false)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_discover_run(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<DiscoverTriggerRequest>,
) -> Response {
    match ctx.run_automated_discovery(body.max_per_seed).await {
        Ok(summary) => Json(summary).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_discover_scan(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<CustomScanConfig>,
) -> Response {
    match ctx.scan_tiktok_custom(&body).await {
        Ok(result) => Json(result).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_list_videos(
    State(ctx): State<Arc<PipelineContext>>,
    Query(q): Query<VideoListQuery>,
) -> Response {
    let filter = VideoFilter {
        lane: q.lane,
        state: q.state,
        verdict: q.verdict,
        label: q.label,
        creator_id: q.creator_id,
        search_query: q.q,
        min_score: q.min_score,
        min_views: q.min_views,
        limit: q.limit,
        offset: q.offset,
    };
    match ctx.db.list_lead_cards(&filter).await {
        Ok(cards) => {
            let cap = ctx.config.read().await.funnel.daily_lead_cap;
            let lane_counts = ctx.db.get_lane_counts().await.unwrap_or_default();
            Json(serde_json::json!({
                "daily_cap": cap,
                "total": cards.len(),
                "lane_counts": lane_counts,
                "cards": cards
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_video_detail(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<String>,
) -> Response {
    match ctx.db.get_video_detail(&id).await {
        Ok(Some(detail)) => Json(detail).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "video not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_delete_video(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<String>,
) -> Response {
    match ctx.db.delete_video(&id).await {
        Ok(()) => {
            ctx.emit(CoreEvent::VideoUpdated {
                video_id: id.clone(),
                state: "DELETED".to_string(),
                lane: None,
            });
            Json(serde_json::json!({ "ok": true, "deleted": id })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_get_deepfake(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<String>,
) -> Response {
    if let Ok(Some(res)) = ctx.db.get_aegis_result(&id).await {
        return Json(serde_json::json!({ "ok": true, "result": res })).into_response();
    }
    match ctx.run_aegis_deepfake_detection(&id).await {
        Ok(res) => Json(serde_json::json!({ "ok": true, "result": res })).into_response(),
        Err(_) => Json(serde_json::json!({ "ok": true, "result": serde_json::Value::Null })).into_response(),
    }
}

struct ActiveTaskCleanup {
    ctx: Arc<PipelineContext>,
    task_id: String,
}

impl Drop for ActiveTaskCleanup {
    fn drop(&mut self) {
        let ctx = Arc::clone(&self.ctx);
        let id = self.task_id.clone();
        tokio::spawn(async move {
            ctx.remove_active_task(&id).await;
        });
    }
}

async fn handle_trigger_deepfake(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<String>,
) -> Response {
    let task_id = format!("aegis_scan:{}", id);
    ctx.upsert_active_task(crate::engine::ActiveResearchTask {
        id: task_id.clone(),
        kind: "aegis_scan".to_string(),
        subject_id: None,
        video_id: Some(id.clone()),
        subject_name: format!("Видео {}", id),
        title: format!("Проверка на записа: видео {}", id),
        stage: "Анализ на аудио и видео поток...".to_string(),
        progress_pct: 45,
        started_at: chrono::Utc::now().to_rfc3339(),
    }).await;
    let _cleanup = ActiveTaskCleanup { ctx: Arc::clone(&ctx), task_id };
    match ctx.run_aegis_deepfake_detection(&id).await {
        Ok(res) => Json(serde_json::json!({ "ok": true, "result": res })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_decision(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<String>,
    Json(body): Json<DecisionRequest>,
) -> Response {
    if !matches!(body.action.as_str(), "keep" | "discard" | "research_more" | "undo" | "rerun_claims" | "rerun_extract" | "rerun_verify") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid decision action" })),
        )
            .into_response();
    }
    match ctx.db.log_decision(&id, &body.action, body.note.as_deref()).await {
        Ok(dec) => {
            if matches!(body.action.as_str(), "research_more" | "rerun_verify") {
                if let Ok(job) = ctx.db.enqueue_job(Some(&id), Stage::ResearchMore).await {
                    let job_id = job.id;
                    let job_stage = job.stage.clone();
                    let run_ctx = Arc::clone(&ctx);
                    let run_vid = id.clone();
                    let _ = ctx.db.mark_job_running(job_id).await;
                    ctx.emit(CoreEvent::JobUpdated {
                        job_id,
                        video_id: Some(id.clone()),
                        stage: job_stage.clone(),
                        status: "running".to_string(),
                    });
                    tokio::spawn(async move {
                        let task_id = format!("video_research:{}", run_vid);
                        run_ctx.upsert_active_task(crate::engine::ActiveResearchTask {
                            id: task_id.clone(),
                            kind: "video_research".to_string(),
                            subject_id: None,
                            video_id: Some(run_vid.clone()),
                            subject_name: format!("Видео {}", run_vid),
                            title: format!("Разширено търсене: видео {}", run_vid),
                            stage: "Проверка на външни източници и регистри...".to_string(),
                            progress_pct: 55,
                            started_at: chrono::Utc::now().to_rfc3339(),
                        }).await;
                        let _cleanup = ActiveTaskCleanup { ctx: Arc::clone(&run_ctx), task_id };
                        let res = run_ctx.run_research_more_job(&run_vid).await;
                        match res {
                            Ok(()) => {
                                let _ = run_ctx.db.complete_job(job_id).await;
                                run_ctx.emit(CoreEvent::JobUpdated {
                                    job_id,
                                    video_id: Some(run_vid.clone()),
                                    stage: job_stage,
                                    status: "succeeded".to_string(),
                                });
                                if let Ok(Some(v)) = run_ctx.db.get_video(&run_vid).await {
                                    run_ctx.emit(CoreEvent::VideoUpdated {
                                        video_id: run_vid,
                                        state: v.state,
                                        lane: v.lane,
                                    });
                                }
                            }
                            Err(err) => {
                                let _ = run_ctx.db.fail_or_retry_job(&job, &err.to_string(), 3, false).await;
                                run_ctx.emit(CoreEvent::JobUpdated {
                                    job_id,
                                    video_id: Some(run_vid),
                                    stage: job_stage,
                                    status: "failed".to_string(),
                                });
                            }
                        }
                    });
                }
            } else if body.action == "rerun_claims" {
                if let Ok(job) = ctx.db.enqueue_job(Some(&id), Stage::Claims).await {
                    let job_id = job.id;
                    let job_stage = job.stage.clone();
                    let run_ctx = Arc::clone(&ctx);
                    let run_vid = id.clone();
                    let _ = ctx.db.mark_job_running(job_id).await;
                    ctx.emit(CoreEvent::JobUpdated {
                        job_id,
                        video_id: Some(id.clone()),
                        stage: job_stage.clone(),
                        status: "running".to_string(),
                    });
                    tokio::spawn(async move {
                        let res = run_ctx.run_claims_job(&run_vid).await;
                        match res {
                            Ok(()) => {
                                let _ = run_ctx.db.complete_job(job_id).await;
                                run_ctx.emit(CoreEvent::JobUpdated {
                                    job_id,
                                    video_id: Some(run_vid.clone()),
                                    stage: job_stage,
                                    status: "succeeded".to_string(),
                                });
                                if let Ok(Some(v)) = run_ctx.db.get_video(&run_vid).await {
                                    run_ctx.emit(CoreEvent::VideoUpdated {
                                        video_id: run_vid,
                                        state: v.state,
                                        lane: v.lane,
                                    });
                                }
                            }
                            Err(err) => {
                                let _ = run_ctx.db.fail_or_retry_job(&job, &err.to_string(), 3, false).await;
                                run_ctx.emit(CoreEvent::JobUpdated {
                                    job_id,
                                    video_id: Some(run_vid),
                                    stage: job_stage,
                                    status: "failed".to_string(),
                                });
                            }
                        }
                    });
                }
            } else if body.action == "rerun_extract" {
                if let Ok(job) = ctx.db.enqueue_job(Some(&id), Stage::Extract).await {
                    ctx.emit(CoreEvent::JobUpdated {
                        job_id: job.id,
                        video_id: Some(id.clone()),
                        stage: job.stage,
                        status: job.status,
                    });
                }
            }
            ctx.emit(CoreEvent::DecisionLogged {
                video_id: id,
                action: body.action,
            });
            Json(dec).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_edit_transcript(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<String>,
    Json(body): Json<TranscriptEditRequest>,
) -> Response {
    if let Err(e) = ctx.db.update_transcript_edited_text(&id, &body.edited_text).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    if body.rerun_claims {
        if let Ok(job) = ctx.db.enqueue_job(Some(&id), Stage::Claims).await {
            ctx.emit(CoreEvent::JobUpdated {
                job_id: job.id,
                video_id: Some(id.clone()),
                stage: job.stage,
                status: job.status,
            });
        }
    }
    if let Ok(Some(detail)) = ctx.db.get_video_detail(&id).await {
        Json(serde_json::to_value(detail).unwrap_or(serde_json::json!({ "ok": true, "video_id": id }))).into_response()
    } else {
        Json(serde_json::json!({ "ok": true, "video_id": id })).into_response()
    }
}

async fn handle_retranscribe(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<String>,
    Json(body): Json<RetranscribeRequest>,
) -> Response {
    if !body.warm_start {
        match ctx.db.enqueue_job(Some(&id), Stage::Extract).await {
            Ok(job) => {
                ctx.emit(CoreEvent::JobUpdated {
                    job_id: job.id,
                    video_id: Some(id.clone()),
                    stage: job.stage,
                    status: job.status,
                });
                Json(serde_json::json!({
                    "ok": true,
                    "video_id": id,
                    "warm_start": false,
                    "enqueued": true
                }))
                .into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        }
    } else {
        let tr_opt = match ctx.db.get_transcript(&id).await {
            Ok(t) => t,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        };

        let Some(tr) = tr_opt else {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Transcript not found" })),
            )
                .into_response();
        };

        let base_text = tr.edited_text.unwrap_or(tr.text);
        let cfg = ctx.config.read().await.clone();
        let lex = leadfinder_providers::global_lexicon(
            &ctx.paths,
            &cfg.normalisation.lexicon_config_file,
        );

        let stage1 = lex.normalize_text(&base_text);

        let system_prompt = "You are a specialized Bulgarian language transcription editor and fact-checking proofreader. You are given an automatic speech recognition (ASR) transcript in Bulgarian with phonetic errors, misrecognized Bulgarian names, distorted courtroom/political terminology, or dropped sounds. Restore the correct Bulgarian text: fix misheard names (e.g. Милен Цветков, Кристиан Николов, Софийски градски съд, прокуратура, затворническа администрация), correct phonetic misspellings, restore proper Bulgarian grammar, punctuation, and capitalization, while keeping the original spoken speech flow intact. Return JSON with a single field 'corrected_text' containing the clean corrected Bulgarian text.".to_string();
        let user_prompt = format!(" what would the fixed text look like?\n{stage1}");

        let runner = leadfinder_providers::LlmChainRunner::from_runtime(&ctx.paths, &cfg, &ctx.db).await;
        let schema_val = serde_json::json!({
            "type": "object",
            "properties": {
                "corrected_text": { "type": "string" }
            },
            "required": ["corrected_text"]
        });

        let llm_req = leadfinder_core::LlmJsonRequest {
            system_prompt,
            user_prompt,
            json_schema: Some(schema_val),
            max_tokens: 1024,
            temperature: 0.1,
        };

        let corrected = match runner.complete_validated(&llm_req, &ctx.db).await {
            Ok(val) => {
                val.response.parsed_json
                    .get("corrected_text")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim().to_string())
                    .unwrap_or(stage1)
            }
            Err(_) => stage1,
        };

        let _ = ctx.db.update_transcript_edited_text(&id, &corrected).await;

        if body.rerun_claims {
            if let Ok(job) = ctx.db.enqueue_job(Some(&id), Stage::Claims).await {
                ctx.emit(CoreEvent::JobUpdated {
                    job_id: job.id,
                    video_id: Some(id.clone()),
                    stage: job.stage,
                    status: job.status,
                });
            }
        }

        Json(serde_json::json!({
            "ok": true,
            "video_id": id,
            "corrected_text": corrected,
            "warm_start": true
        }))
        .into_response()
    }
}

async fn handle_ai_check_transcript(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<String>,
) -> Response {
    let tr_opt = match ctx.db.get_transcript(&id).await {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let Some(tr) = tr_opt else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Transcript not found" })),
        )
            .into_response();
    };

    let base_text = tr.edited_text.unwrap_or(tr.text);
    let cfg = ctx.config.read().await.clone();
    let lex = leadfinder_providers::global_lexicon(
        &ctx.paths,
        &cfg.normalisation.lexicon_config_file,
    );

    let stage1 = lex.normalize_text(&base_text);

    let system_prompt = "You are a specialized Bulgarian language transcription editor and fact-checking proofreader. You are given an automatic speech recognition (ASR) transcript in Bulgarian with phonetic errors, misrecognized Bulgarian names, distorted courtroom/political terminology, or dropped sounds. Restore the correct Bulgarian text: fix misheard names (e.g. Милен Цветков, Кристиан Николов, Софийски градски съд, прокуратура, затворническа администрация), correct phonetic misspellings, restore proper Bulgarian grammar, punctuation, and capitalization, while keeping the original spoken speech flow intact. Return JSON with a single field 'corrected_text' containing the clean corrected Bulgarian text.".to_string();
    let user_prompt = format!(" what would the fixed text look like?\n{stage1}");

    let runner = leadfinder_providers::LlmChainRunner::from_runtime(&ctx.paths, &cfg, &ctx.db).await;
    let schema_val = serde_json::json!({
        "type": "object",
        "properties": {
            "corrected_text": { "type": "string" }
        },
        "required": ["corrected_text"]
    });

    let llm_req = leadfinder_core::LlmJsonRequest {
        system_prompt,
        user_prompt,
        json_schema: Some(schema_val),
        max_tokens: 1024,
        temperature: 0.1,
    };

    let corrected = match runner.complete_validated(&llm_req, &ctx.db).await {
        Ok(val) => {
            val.response.parsed_json
                .get("corrected_text")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .unwrap_or(stage1)
        }
        Err(_) => stage1,
    };

    let _ = ctx.db.update_transcript_edited_text(&id, &corrected).await;

    Json(serde_json::json!({
        "ok": true,
        "video_id": id,
        "corrected_text": corrected
    }))
    .into_response()
}

fn resolve_headless_scraping_browser() -> Option<String> {
    if cfg!(target_os = "windows") {
        let candidates = [
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ];
        for c in candidates {
            if std::path::Path::new(c).exists() {
                return Some(c.to_string());
            }
        }
    }
    None
}

async fn run_google_ai_mode_fix(raw_text: &str) -> Option<String> {
    let prompt = format!("what would the fixed text look like? Output the edited result only:{}", raw_text.trim());

    let py_script = if std::path::Path::new("scripts/google_ai_search.py").exists() {
        Some("scripts/google_ai_search.py")
    } else if std::path::Path::new("tools/google_ai_search.py").exists() {
        Some("tools/google_ai_search.py")
    } else {
        None
    };

    if let Some(script_path) = py_script {
        if let Ok(py_out) = tokio::process::Command::new("python")
            .env("PYTHONIOENCODING", "utf-8")
            .arg(script_path)
            .arg("--json")
            .arg("--mode")
            .arg("refine")
            .arg(&prompt)
            .output()
            .await
        {
            if py_out.status.success() {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&py_out.stdout) {
                    if val.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                        if let Some(t) = val.get("text").and_then(|v| v.as_str()) {
                            let clean = t.trim();
                            if !clean.is_empty() {
                                return Some(clean.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    let encoded_query: String = url::form_urlencoded::byte_serialize(prompt.as_bytes()).collect();
    let url = format!("https://www.google.com/search?udm=50&q={}", encoded_query);

    let Some(bin) = resolve_headless_scraping_browser() else {
        return None;
    };

    let app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| "data".to_string());
    let session_dir = std::path::PathBuf::from(app_data).join("LeadFinder").join("google_session");
    let _ = std::fs::create_dir_all(&session_dir);
    let session_arg = format!("--user-data-dir={}", session_dir.display());

    let output = tokio::process::Command::new(&bin)
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-sandbox")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-blink-features=AutomationControlled")
        .arg(&session_arg)
        .arg("--virtual-time-budget=10000")
        .arg("--dump-dom")
        .arg(&url)
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let dom = String::from_utf8_lossy(&output.stdout);
    extract_fixed_text_from_google_dom(&dom)
}

fn extract_fixed_text_from_google_dom(dom: &str) -> Option<String> {
    if let Ok(re_quotes) = regex::Regex::new(r#"[„"“]([А-ЯA-Z][^"“”]{15,})[“"”]"#) {
        if let Some(caps) = re_quotes.captures(dom) {
            if let Some(m) = caps.get(1) {
                let res = m.as_str().trim();
                if !res.is_empty() {
                    return Some(res.to_string());
                }
            }
        }
    }

    let re_block = regex::Regex::new(r#"data-processed="true">(.*?)</div>"#).ok()?;
    let blocks: Vec<&str> = re_block.find_iter(dom).map(|m| m.as_str()).collect();
    let re_tag = regex::Regex::new(r#"<[^>]+>"#).ok()?;
    for (i, b) in blocks.iter().enumerate() {
        let clean = re_tag.replace_all(b, " ");
        let s = clean.trim();
        if s.contains("би изглеждал така") || s.contains("вариант на текста") {
            if i + 1 < blocks.len() {
                let nxt_clean = re_tag.replace_all(blocks[i + 1], " ");
                let nxt = nxt_clean.trim().trim_matches(|c| c == '„' || c == '"' || c == '”' || c == '“');
                if nxt.chars().count() > 10 {
                    return Some(nxt.to_string());
                }
            }
        }
    }
    None
}

fn format_timed_segments_for_dialogue(
    lex: &leadfinder_providers::LexiconNormalizer,
    segments: &[leadfinder_core::TranscriptSegment],
    fallback: &str,
) -> String {
    let normalized_fallback = lex.normalize_text(fallback);
    if segments.is_empty() {
        return normalized_fallback;
    }

    let mut out = String::new();
    let mut current_sec_bucket: i64 = -1;

    for seg in segments {
        let sec = seg.start_ms / 1000;
        let bucket = (sec / 45) * 45;
        if bucket != current_sec_bucket {
            current_sec_bucket = bucket;
            let m = sec / 60;
            let s = sec % 60;
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&format!("**{:02}:{:02}**\n", m, s));
        }

        let cleaned = lex.normalize_text(&seg.text);
        let trimmed = cleaned.trim();
        if trimmed.is_empty() {
            continue;
        }

        out.push_str(&format!("{}\n", trimmed));
    }

    if out.trim().is_empty() {
        normalized_fallback
    } else {
        out.trim().to_string()
    }
}

async fn handle_google_ai_fix(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<String>,
) -> Response {
    let tr_opt = match ctx.db.get_transcript(&id).await {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let Some(tr) = tr_opt else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Transcript not found" })),
        )
            .into_response();
    };

    let cfg = ctx.config.read().await.clone();
    let lex = leadfinder_providers::global_lexicon(
        &ctx.paths,
        &cfg.normalisation.lexicon_config_file,
    );

    let base_text = tr.edited_text.unwrap_or(tr.text);
    let timed_raw = if !tr.segments.is_empty() {
        let mut s = String::new();
        let mut cur_bucket: i64 = -1;
        for seg in &tr.segments {
            let sec = seg.start_ms / 1000;
            let bucket = (sec / 45) * 45;
            if bucket != cur_bucket {
                cur_bucket = bucket;
                let m = sec / 60;
                let s_sec = sec % 60;
                if !s.is_empty() {
                    s.push_str("\n\n");
                }
                s.push_str(&format!("**{:02}:{:02}**\n", m, s_sec));
            }
            let t = seg.text.trim();
            if !t.is_empty() {
                s.push_str(t);
                s.push(' ');
            }
        }
        s.trim().to_string()
    } else {
        base_text.clone()
    };

    let maybe_fixed = run_google_ai_mode_fix(&timed_raw).await;

    let final_text = match maybe_fixed {
        Some(t) if !t.trim().is_empty() => t,
        _ => {
            let stage1 = lex.normalize_text(&timed_raw);
            let runner = leadfinder_providers::LlmChainRunner::from_runtime(&ctx.paths, &cfg, &ctx.db).await;
            let schema_val = serde_json::json!({
                "type": "object",
                "properties": {
                    "corrected_text": { "type": "string" }
                },
                "required": ["corrected_text"]
            });
            let llm_req = leadfinder_core::LlmJsonRequest {
                system_prompt: "You are an expert Bulgarian transcript editor and proofreader. Given an automatic speech recognition (ASR) transcript in Bulgarian with timestamps and phonetic errors, restore the clean Bulgarian text formatted in flowing paragraphs with bold timestamps (**MM:SS**). Fix all misheard Bulgarian words (e.g. алкогол -> алкохол, кота -> колата, полезай -> полицаи, и 12 -> 112, бърза полнош -> Бърза помощ, нуа нуа -> 0.0), restore Bulgarian grammar, punctuation, and question marks. Do not invent fake or arbitrary speaker names. Return JSON with 'corrected_text'.".to_string(),
                user_prompt: format!("what would the fixed text look like? Output the edited result only:\n{}", stage1),
                json_schema: Some(schema_val),
                max_tokens: 8192,
                temperature: 0.1,
            };
            match runner.complete_validated(&llm_req, &ctx.db).await {
                Ok(val) => val.response.parsed_json.get("corrected_text").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_else(|| format_timed_segments_for_dialogue(&lex, &tr.segments, &base_text)),
                Err(_) => format_timed_segments_for_dialogue(&lex, &tr.segments, &base_text),
            }
        }
    };

    let _ = ctx.db.update_transcript_edited_text(&id, &final_text).await;

    Json(serde_json::json!({
        "ok": true,
        "video_id": id,
        "corrected_text": final_text,
        "source": "google_ai_mode"
    }))
    .into_response()
}

async fn handle_google_audio_transcribe(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<String>,
) -> Response {
    let py_script = if std::path::Path::new("scripts/google_audio_transcribe.py").exists() {
        Some("scripts/google_audio_transcribe.py")
    } else if std::path::Path::new("tools/google_audio_transcribe.py").exists() {
        Some("tools/google_audio_transcribe.py")
    } else {
        None
    };

    let Some(script_path) = py_script else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "Transcription script not found" })),
        )
            .into_response();
    };

    let cfg = ctx.config.read().await.clone();
    let lex = leadfinder_providers::global_lexicon(
        &ctx.paths,
        &cfg.normalisation.lexicon_config_file,
    );

    let py_res = tokio::process::Command::new("python")
        .env("PYTHONIOENCODING", "utf-8")
        .arg(script_path)
        .arg("--video-id")
        .arg(&id)
        .output()
        .await;

    let output = match py_res {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            let err_msg = String::from_utf8_lossy(&o.stderr);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Script failed: {err_msg}") })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let val: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Invalid JSON output: {e}") })),
            )
                .into_response();
        }
    };

    let corrected = val
        .get("corrected_text")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    let normalized = lex.normalize_text(corrected);

    let parsed_segs = val
        .get("segments")
        .and_then(|v| v.as_array())
        .and_then(|arr| serde_json::from_value::<Vec<leadfinder_core::TranscriptSegment>>(serde_json::Value::Array(arr.clone())).ok())
        .unwrap_or_default();

    if let Ok(Some(mut tr)) = ctx.db.get_transcript(&id).await {
        tr.edited_text = Some(normalized.clone());
        if !parsed_segs.is_empty() {
            tr.segments = parsed_segs;
        }
        let _ = ctx.db.save_transcript(&tr).await;
    } else {
        let _ = ctx.db.update_transcript_edited_text(&id, &normalized).await;
    }

    Json(serde_json::json!({
        "ok": true,
        "video_id": id,
        "corrected_text": normalized,
        "source": val.get("source").and_then(|v| v.as_str()).unwrap_or("google_audio")
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct GoogleAiQueryRequest {
    pub query: String,
    pub mode: Option<String>,
}

async fn handle_google_ai_query(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<GoogleAiQueryRequest>,
) -> Response {
    let mode = body.mode.unwrap_or_else(|| "research".to_string());
    let query = body.query.trim().to_string();
    if query.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Query cannot be empty" })),
        )
            .into_response();
    }

    let py_script = if std::path::Path::new("scripts/google_ai_search.py").exists() {
        Some("scripts/google_ai_search.py")
    } else if std::path::Path::new("tools/google_ai_search.py").exists() {
        Some("tools/google_ai_search.py")
    } else {
        None
    };

    if let Some(script_path) = py_script {
        if let Ok(py_out) = tokio::process::Command::new("python")
            .arg(script_path)
            .arg("--json")
            .arg("--mode")
            .arg(&mode)
            .arg(&query)
            .output()
            .await
        {
            if py_out.status.success() {
                if let Ok(mut val) = serde_json::from_slice::<serde_json::Value>(&py_out.stdout) {
                    if val.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                        if let Some(obj) = val.as_object_mut() {
                            obj.insert("source".to_string(), serde_json::json!("google_ai_mode"));
                        }
                        return Json(val).into_response();
                    }
                }
            }
        }
    }

    let cfg = ctx.config.read().await.clone();
    let runner = leadfinder_providers::LlmChainRunner::from_runtime(&ctx.paths, &cfg, &ctx.db).await;

    if mode == "refine" {
        let lex = leadfinder_providers::global_lexicon(
            &ctx.paths,
            &cfg.normalisation.lexicon_config_file,
        );
        let stage1 = lex.normalize_text(&query);
        let schema_val = serde_json::json!({
            "type": "object",
            "properties": {
                "corrected_text": { "type": "string" }
            },
            "required": ["corrected_text"]
        });
        let llm_req = leadfinder_core::LlmJsonRequest {
            system_prompt: "You are a specialized Bulgarian language transcription editor and fact-checking proofreader. You are given an automatic speech recognition (ASR) transcript in Bulgarian with phonetic errors, misrecognized Bulgarian names, distorted courtroom/political terminology, or dropped sounds. Restore the correct Bulgarian text: fix misheard names (e.g. Милен Цветков, Кристиан Николов, Софийски градски съд, прокуратура, затворническа администрация), correct phonetic misspellings, restore proper Bulgarian grammar, punctuation, and capitalization, while keeping the original spoken speech flow intact. Return JSON with a single field 'corrected_text' containing the clean corrected Bulgarian text.".to_string(),
            user_prompt: format!("what would the fixed text look like? Output the edited result only:{stage1}"),
            json_schema: Some(schema_val),
            max_tokens: 1024,
            temperature: 0.1,
        };
        let fixed = match runner.complete_validated(&llm_req, &ctx.db).await {
            Ok(v) => v.response.parsed_json.get("corrected_text").and_then(|s| s.as_str()).map(|s| s.to_string()).unwrap_or(stage1),
            Err(_) => stage1,
        };
        return Json(serde_json::json!({
            "ok": true,
            "query": query,
            "mode": mode,
            "text": fixed,
            "citations": [],
            "source": "local_llm_fallback"
        })).into_response();
    }

    let primary_entries = leadfinder_core::load_primary_sources(&ctx.paths.config_dir).unwrap_or_default();
    let search_engine = leadfinder_providers::WebAndPrimarySearch::new(
        &cfg.providers.searxng_url,
        primary_entries,
        cfg.factcheck_archives.clone(),
        cfg.verification.news_domains.clone(),
        ctx.guard.clone(),
    );
    let lex = leadfinder_providers::global_lexicon(&ctx.paths, &cfg.normalisation.lexicon_config_file);
    let ranker = leadfinder_providers::PassageExtractorAndRanker::new(ctx.guard.clone(), lex);
    let search_q = leadfinder_core::SearchQuery {
        query: format!("{} факти новини проверка източници", query),
        restrict_domains: Vec::new(),
        date_from: None,
        date_to: None,
        max_results: 5,
    };
    let mut citations = Vec::new();
    let mut text_snippets = Vec::new();
    if let Ok(hits) = search_engine.search(&search_q).await {
        for hit in hits {
            citations.push(serde_json::json!({ "title": hit.title, "url": hit.url }));
            let html = ranker.fetch_html_cached(&hit.url).await.unwrap_or_default();
            let txt = ranker.extract_main_text(&html);
            if !txt.is_empty() {
                text_snippets.push(format!("Източник {}: {}\n{}", hit.title, hit.url, txt.chars().take(600).collect::<String>()));
            }
        }
    }

    let prompt_research = format!(
        "Тема за проучване: {}\n\nНамерени първични източници:\n{}\n\nСинтезирай изчерпателен обективен аналитичен конспект на български език с цитиране на източниците.",
        query,
        text_snippets.join("\n\n")
    );
    let schema_val = serde_json::json!({
        "type": "object",
        "properties": {
            "summary": { "type": "string" }
        },
        "required": ["summary"]
    });
    let llm_req = leadfinder_core::LlmJsonRequest {
        system_prompt: "Ти си обективен журналистически анализатор и фактчекинг изследовател. Синтезирай точен конспект с факти, контекст и източници.".to_string(),
        user_prompt: prompt_research,
        json_schema: Some(schema_val),
        max_tokens: 1500,
        temperature: 0.1,
    };
    let summary = match runner.complete_validated(&llm_req, &ctx.db).await {
        Ok(v) => v.response.parsed_json.get("summary").and_then(|s| s.as_str()).map(|s| s.to_string()).unwrap_or_else(|| "Няма допълнителни данни.".to_string()),
        Err(_) => "Няма допълнителни данни.".to_string(),
    };

    Json(serde_json::json!({
        "ok": true,
        "query": query,
        "mode": mode,
        "text": summary,
        "citations": citations,
        "source": "searxng_llm_fallback"
    })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct EvidencePreviewQuery {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct ResearchPersonRequest {
    pub name: Option<String>,
}

async fn handle_evidence_preview(
    State(ctx): State<Arc<PipelineContext>>,
    Query(q): Query<EvidencePreviewQuery>,
) -> Response {
    let cfg = ctx.config.read().await.clone();
    let lex = leadfinder_providers::global_lexicon(&ctx.paths, &cfg.normalisation.lexicon_config_file);
    let ranker = leadfinder_providers::PassageExtractorAndRanker::new(ctx.guard.clone(), lex);
    let body = ranker.fetch_html_cached(&q.url).await.unwrap_or_default();
    let text = ranker.extract_main_text(&body);
    let title = if let Some(start) = body.to_lowercase().find("<title>") {
        if let Some(end) = body.to_lowercase().find("</title>") {
            if end > start + 7 {
                body[start + 7..end].trim().to_string()
            } else {
                q.url.clone()
            }
        } else {
            q.url.clone()
        }
    } else {
        q.url.clone()
    };
    let domain = url::Url::parse(&q.url).ok().and_then(|u| u.domain().map(|d| d.to_string())).unwrap_or_default();
    Json(serde_json::json!({
        "url": q.url,
        "title": title,
        "text": text,
        "domain": domain
    })).into_response()
}

async fn handle_delete_evidence(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<i64>,
) -> Response {
    match ctx.db.delete_evidence(id).await {
        Ok(true) => Json(serde_json::json!({ "ok": true, "deleted_id": id })).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "evidence not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_open_browser(
    Json(body): Json<OpenBrowserRequest>,
) -> Response {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", &body.url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(&body.url).spawn();
    }
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn handle_research_person(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<i64>,
    Json(body): Json<ResearchPersonRequest>,
) -> Response {
    let (existing_person, existing_cat, existing_ent, person_variants, db_name) =
        if let Ok(Some(p)) = ctx.db.get_person_by_id(id).await {
            let vars = p.variants.clone();
            let cname = p.canonical_name.clone();
            (Some(p.clone()), p.category, Some(p.entity_type), vars, cname)
        } else {
            (None, None, None, Vec::new(), String::new())
        };

    let name = body
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(db_name);
    let entity_type = existing_ent
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| leadfinder_providers::detect_entity_type(&name, None).to_string());

    let task_id = format!("person_research:{}", id);
    ctx.upsert_active_task(crate::engine::ActiveResearchTask {
        id: task_id.clone(),
        kind: "person_research".to_string(),
        subject_id: Some(id),
        video_id: None,
        subject_name: name.clone(),
        title: format!("Проучване на обект: {}", name),
        stage: "Проверка на регистри, архиви и новини".to_string(),
        progress_pct: 12,
        started_at: chrono::Utc::now().to_rfc3339(),
    })
    .await;
    let _task_guard = ActiveTaskCleanup {
        ctx: ctx.clone(),
        task_id: task_id.clone(),
    };

    let cfg = ctx.config.read().await.clone();
    let lex = leadfinder_providers::global_lexicon(&ctx.paths, &cfg.normalisation.lexicon_config_file);

    let archive_ep = cfg.factcheck_archives.clone();
    let primary_entries = leadfinder_core::load_primary_sources(&ctx.paths.config_dir).unwrap_or_default();
    let search_engine = leadfinder_providers::WebAndPrimarySearch::new(
        &cfg.providers.searxng_url,
        primary_entries,
        archive_ep,
        cfg.verification.news_domains.clone(),
        ctx.guard.clone(),
    );
    let ranker = leadfinder_providers::PassageExtractorAndRanker::new(ctx.guard.clone(), lex);
    let mut scraped_docs: Vec<ScrapedOsintDoc> = Vec::new();

    let py_script = if std::path::Path::new("scripts/google_ai_search.py").exists() {
        Some("scripts/google_ai_search.py")
    } else if std::path::Path::new("tools/google_ai_search.py").exists() {
        Some("tools/google_ai_search.py")
    } else {
        None
    };

    if let Some(script_path) = py_script {
        if let Ok(Ok(py_out)) = tokio::time::timeout(
            std::time::Duration::from_secs(12),
            tokio::process::Command::new("python")
                .arg(script_path)
                .arg("--json")
                .arg("--mode")
                .arg("research")
                .arg(&name)
                .output(),
        )
        .await
        {
            if py_out.status.success() {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&py_out.stdout) {
                    if let Some(ai_txt) = val.get("text").and_then(|v| v.as_str()) {
                        let clean = ai_txt.trim();
                        if !clean.is_empty() {
                            scraped_docs.push(ScrapedOsintDoc {
                                bucket: "gov".to_string(),
                                title: format!("Обобщена уеб справка за {}", name),
                                url: String::new(),
                                text: clean.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    let sanctions_matcher = leadfinder_providers::SanctionsMatcher::new();
    let news_stream = leadfinder_providers::DynamicNewsStream::new();
    let (sanctions_res, news_res) = tokio::join!(
        sanctions_matcher.check_entity(&name, &person_variants),
        news_stream.fetch_events_for_entity(id, &name, &person_variants, 10)
    );

    let sanctions_matches = sanctions_res.unwrap_or_default();
    let _ = ctx.db.save_sanctions_matches(id, &sanctions_matches).await;

    let mut sanctions_summary_lines: Vec<String> = Vec::new();
    for sm in &sanctions_matches {
        let mut line = format!("- [{}] {} (Съвпадение: {:.0}%)", sm.dataset, sm.caption, sm.match_score * 100.0);
        if let Some(ref p) = sm.program {
            line.push_str(&format!(" | Програма: {}", p));
        }
        if let Some(ref s) = sm.sanctor {
            line.push_str(&format!(" | Орган: {}", s));
        }
        sanctions_summary_lines.push(line);
    }
    let sanctions_context_block = if sanctions_summary_lines.is_empty() {
        "Проверени санкционни регистри (ООН, US OFAC SDN, ЕС, UK OFSI, SECO, НАЗК, Интерпол): не са установени активни вписвания.".to_string()
    } else {
        format!("Вписвания в международни санкционни и PEP регистри:\n{}", sanctions_summary_lines.join("\n"))
    };

    let live_events = news_res.unwrap_or_default();
    let mut newly_recorded_events = 0usize;
    for ev in &live_events {
        if ctx
            .db
            .record_intel_event(id, &ev.source_name, &ev.title, &ev.url, &ev.snippet)
            .await
            .is_ok()
        {
            newly_recorded_events += 1;
        }
        scraped_docs.push(ScrapedOsintDoc {
            bucket: "media".to_string(),
            title: format!("[{}] {}", ev.source_name, ev.title),
            url: ev.url.clone(),
            text: format!("{}. {}", ev.title, ev.snippet),
        });
    }
    let _ = ctx.db.touch_person_monitored(id).await;

    ctx.update_active_task(&task_id, "Справка в архивни бази и Уикипедия", 32)
        .await;

    let candidate_archive_urls: Vec<String> = live_events.iter().map(|e| e.url.clone()).collect();
    let web_archive_engine = leadfinder_providers::WebArchiveEngine::new();
    let temporal_audit = web_archive_engine
        .run_aegis_temporal_audit(&name, &entity_type, &candidate_archive_urls)
        .await;

    let mut archive_summary_lines: Vec<String> = Vec::new();
    archive_summary_lines.push(format!(
        "- Състояние на архивната следа: {}",
        temporal_audit.scrubbed_or_altered_risk
    ));
    for note in &temporal_audit.provenance_notes {
        archive_summary_lines.push(format!("- {}", note));
    }
    for snap in &temporal_audit.historical_snapshots {
        archive_summary_lines.push(format!(
            "- [Wayback Machine] {} -> {} (HTTP {})",
            snap.timestamp, snap.url, snap.status
        ));
    }
    for doc in &temporal_audit.internet_archive_docs {
        archive_summary_lines.push(format!(
            "- [Internet Archive] {} ({}) -> {}",
            doc.title,
            doc.date.as_deref().unwrap_or("без дата"),
            doc.detail_url
        ));
    }
    let archive_context_block = format!(
        "Архивни копия и история на публикациите (Wayback Machine / Internet Archive / Уикипедия):\n{}",
        archive_summary_lines.join("\n")
    );

    let mut video_contexts: Vec<String> = Vec::new();
    let mut aegis_dossier_notes: Vec<String> = Vec::new();
    if let Ok(vids) = ctx.db.get_videos_for_person(id).await {
        for (vid_id, caption, handle) in vids {
            let mut line = format!("Видео ID: {}", vid_id);
            if let Some(ref h) = handle {
                if !h.is_empty() {
                    line.push_str(&format!(" (Автор: @{})", h));
                }
            }
            if let Some(ref c) = caption {
                if !c.is_empty() {
                    line.push_str(&format!(" | Описание: {}", c));
                }
            }
            if let Ok(Some(vdetail)) = ctx.db.get_video_detail(&vid_id).await {
                for cwe in vdetail.claims.iter().take(6) {
                    let verdict_tag = cwe.claim.verdict.as_deref().unwrap_or("НЕПРОВЕРЕНО");
                    line.push_str(&format!(
                        "\n  • Твърдение [{}]: {}",
                        verdict_tag, cwe.claim.text_bg
                    ));
                    for ev in cwe.evidence.iter().take(3) {
                        if !ev.source_url.is_empty()
                            && ev.passage.chars().count() >= 35
                            && !scraped_docs.iter().any(|d| d.url == ev.source_url)
                        {
                            scraped_docs.push(ScrapedOsintDoc {
                                bucket: "media".to_string(),
                                title: ev
                                    .source_name
                                    .clone()
                                    .unwrap_or_else(|| "Доказателствен източник".to_string()),
                                url: ev.source_url.clone(),
                                text: ev.passage.clone(),
                            });
                        }
                    }
                }
            }
            video_contexts.push(line);

            if let Ok(Some(df)) = ctx.db.get_aegis_result(&vid_id).await {
                let v_up = df.overall_verdict.to_uppercase();
                let is_df = v_up == "DEEPFAKE"
                    || v_up == "CONFIRMED_DEEPFAKE"
                    || v_up == "AI_GENERATED_VIDEO"
                    || v_up == "AI_VOICE_CLONE"
                    || v_up == "ROBOTIC_TTS"
                    || v_up == "SUSPECTED_SYNTHETIC"
                    || v_up == "SUSPICIOUS";
                if is_df || df.audio.synthetic || df.video.synthetic {
                    let mut note = format!("- Видео ID {}: оценка '{}'.", vid_id, df.overall_verdict);
                    if df.audio.synthetic {
                        note.push_str(&format!(
                            " Аудио: {:.0}% ({})",
                            df.audio.score * 100.0,
                            df.audio.classification
                        ));
                        if let Some(ref t) = df.audio.target_person {
                            note.push_str(&format!(" [Глас: {}]", t));
                        }
                    }
                    if df.video.synthetic {
                        note.push_str(&format!(
                            " Картина: {:.0}% ({})",
                            df.video.score * 100.0,
                            df.video.classification
                        ));
                        if let Some(ref t) = df.video.target_person {
                            note.push_str(&format!(" [Лице: {}]", t));
                        }
                    }
                    aegis_dossier_notes.push(note);
                }
            }
        }
    }

    let vid_context_block = if video_contexts.is_empty() {
        "Няма свързани локални видеа.".to_string()
    } else {
        video_contexts.join("\n")
    };

    let aegis_summary_str = if aegis_dossier_notes.is_empty() {
        "В проверените видеа в системата не са установени синтетично генерирани аудио или видео манипулации, свързани с обекта.".to_string()
    } else {
        format!("Установени синтетични или манипулирани видеозаписи:\n{}", aegis_dossier_notes.join("\n"))
    };

    ctx.update_active_task(
        &task_id,
        "Проверка на Търговския регистър, поръчки и съдебни актове",
        50,
    )
    .await;

    let bucketed_queries: Vec<(&str, String)> = match entity_type.as_str() {
        "infrastructure" => vec![
            ("fin", format!("\"{}\" поръчка OR договор OR ЦАИС", name)),
            ("fin", format!("\"{}\" стойност OR бюджет OR милиона", name)),
            ("net", format!("\"{}\" изпълнител OR консорциум OR ДЗЗД", name)),
            ("net", format!("\"{}\" строител OR надзор OR ЕИК", name)),
            ("gov", format!("\"{}\" МРРБ OR АПИ OR \"Акт 16\"", name)),
            ("legal", format!("\"{}\" ДНСК OR КЗК OR ВАС OR проверка", name)),
            ("media", format!("\"{}\" разследване OR ремонт OR сигнали", name)),
        ],
        "building" => vec![
            ("fin", format!("\"{}\" собственост OR имот OR сделка", name)),
            ("fin", format!("\"{}\" поръчка OR договор OR стойност", name)),
            ("net", format!("\"{}\" инвеститор OR строител OR собственик", name)),
            ("net", format!("\"{}\" дружество OR ЕООД OR ООД OR ЕИК", name)),
            ("gov", format!("\"{}\" \"разрешение за строеж\" OR ПУП OR \"Акт 16\"", name)),
            ("legal", format!("\"{}\" ДНСК OR ВАС OR съд OR глоба", name)),
            ("media", format!("\"{}\" разследване OR реконструкция", name)),
        ],
        "institution" => vec![
            ("fin", format!("\"{}\" бюджет OR поръчки OR договори", name)),
            ("fin", format!("\"{}\" \"Сметна палата\" OR одит OR ЦАИС", name)),
            ("net", format!("\"{}\" ръководство OR директор OR \"управителен съвет\"", name)),
            ("gov", format!("\"{}\" закон OR отчети OR решения", name)),
            ("legal", format!("\"{}\" ВАС OR дело OR проверки OR санкции", name)),
            ("media", format!("\"{}\" разследване OR позиции", name)),
        ],
        "group" => vec![
            ("fin", format!("\"{}\" финансиране OR субсидия OR дарения", name)),
            ("fin", format!("\"{}\" \"Сметна палата\" OR отчет OR ЕИК", name)),
            ("net", format!("\"{}\" председател OR ръководство OR коалиция", name)),
            ("gov", format!("\"{}\" избори OR мандати OR парламент", name)),
            ("legal", format!("\"{}\" ЦИК OR съд OR регистрация", name)),
            ("media", format!("\"{}\" разследване OR позиции", name)),
        ],
        "channel" => vec![
            ("fin", format!("\"{}\" собственост OR издател OR ЕИК", name)),
            ("fin", format!("\"{}\" ЕООД OR ООД OR АД OR СЕМ", name)),
            ("net", format!("\"{}\" собственик OR продуцент OR редакция", name)),
            ("gov", format!("\"{}\" лиценз OR СЕМ OR регистър", name)),
            ("legal", format!("\"{}\" СЕМ OR решение OR съд OR опровержение", name)),
            ("media", format!("\"{}\" проверка OR анализ", name)),
        ],
        "country" => vec![
            ("fin", format!("\"{}\" БВП OR бюджет OR дълг OR търговия", name)),
            ("net", format!("\"{}\" правителство OR президент OR премиер", name)),
            ("gov", format!("\"{}\" институции OR управление OR конституция", name)),
            ("legal", format!("\"{}\" договори OR санкции OR съд", name)),
            ("media", format!("\"{}\" икономика OR анализ", name)),
        ],
        _ => vec![
            ("fin", format!("\"{}\" ЕИК OR ООД OR ЕООД OR АД", name)),
            ("fin", format!("\"{}\" papagal OR \"търговски регистър\" OR дружество", name)),
            ("fin", format!("\"{}\" декларация OR КПКОНПИ OR \"Сметна палата\"", name)),
            ("net", format!("\"{}\" съдружник OR управител OR \"свързани лица\"", name)),
            ("gov", format!("\"{}\" биография OR длъжност OR назначение", name)),
            ("legal", format!("\"{}\" ВАС OR СЕС OR дело OR несъвместимост", name)),
            ("media", format!("\"{}\" разследване OR bird OR капитал", name)),
        ],
    };

    let name_low = name.to_lowercase();
    let name_tokens: Vec<String> = name_low
        .split_whitespace()
        .filter(|t| t.chars().count() >= 3)
        .map(|s| s.to_string())
        .collect();

    for (bucket, q_str) in bucketed_queries {
        let sq = leadfinder_core::SearchQuery {
            query: q_str,
            restrict_domains: Vec::new(),
            date_from: None,
            date_to: None,
            max_results: 4,
        };
        if let Ok(Ok(hits)) =
            tokio::time::timeout(std::time::Duration::from_secs(7), search_engine.search(&sq)).await
        {
            for hit in hits.into_iter().take(3) {
                if scraped_docs.iter().any(|d| !d.url.is_empty() && d.url == hit.url) {
                    continue;
                }
                let html = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    ranker.fetch_html_cached(&hit.url),
                )
                .await
                .ok()
                .flatten()
                .unwrap_or_default();
                let extracted = ranker.extract_main_text(&html);
                let body_text = if extracted.chars().count() >= 140 {
                    if hit.snippet.trim().is_empty() {
                        extracted.chars().take(3800).collect::<String>()
                    } else {
                        format!(
                            "{}\n{}",
                            hit.snippet.trim(),
                            extracted.chars().take(3500).collect::<String>()
                        )
                    }
                } else {
                    format!("{} {}", hit.title.trim(), hit.snippet.trim())
                };
                if body_text.trim().chars().count() < 25 {
                    continue;
                }
                let combined_low = format!("{} {}", hit.title.to_lowercase(), body_text.to_lowercase());
                let relevant = if name_tokens.len() >= 2 {
                    combined_low.contains(&name_low)
                        || name_tokens.iter().filter(|t| combined_low.contains(t.as_str())).count() >= 1
                } else {
                    combined_low.contains(&name_low)
                };
                if relevant {
                    scraped_docs.push(ScrapedOsintDoc {
                        bucket: bucket.to_string(),
                        title: hit.title,
                        url: hit.url,
                        text: body_text,
                    });
                }
            }
        }
    }

    let wiki_full_text = temporal_audit
        .wiki_bio
        .as_ref()
        .map(|w| w.extract.as_str())
        .unwrap_or_default();

    let mut preliminary_corpus = String::new();
    preliminary_corpus.push_str(wiki_full_text);
    for doc in &scraped_docs {
        preliminary_corpus.push('\n');
        preliminary_corpus.push_str(&doc.title);
        preliminary_corpus.push('\n');
        preliminary_corpus.push_str(&doc.text);
    }

    let mut entity_pivots = leadfinder_providers::extract_entity_pivots(&preliminary_corpus, &name);

    if !entity_pivots.is_empty() {
        ctx.update_active_task(
            &task_id,
            "Проверка на свързани търговски дружества и ЕИК",
            66,
        )
        .await;
    }

    for pivot in entity_pivots.iter().take(3) {
        let pivot_query = format!(
            "\"{}\" ЕИК OR управител OR съдружник",
            pivot.entity_name
        );
        let sq = leadfinder_core::SearchQuery {
            query: pivot_query,
            restrict_domains: Vec::new(),
            date_from: None,
            date_to: None,
            max_results: 3,
        };
        if let Ok(Ok(hits)) =
            tokio::time::timeout(std::time::Duration::from_secs(6), search_engine.search(&sq)).await
        {
            for hit in hits.into_iter().take(2) {
                if scraped_docs.iter().any(|d| !d.url.is_empty() && d.url == hit.url) {
                    continue;
                }
                let html = tokio::time::timeout(
                    std::time::Duration::from_secs(4),
                    ranker.fetch_html_cached(&hit.url),
                )
                .await
                .ok()
                .flatten()
                .unwrap_or_default();
                let extracted = ranker.extract_main_text(&html);
                let body_text = if extracted.chars().count() >= 120 {
                    format!(
                        "{}\n{}",
                        hit.snippet.trim(),
                        extracted.chars().take(2800).collect::<String>()
                    )
                } else {
                    format!("{} {}", hit.title.trim(), hit.snippet.trim())
                };
                if body_text.trim().chars().count() >= 25 {
                    scraped_docs.push(ScrapedOsintDoc {
                        bucket: "pivot".to_string(),
                        title: format!("[{}] {}", pivot.entity_name, hit.title),
                        url: hit.url,
                        text: body_text,
                    });
                }
            }
        }
    }

    let mut full_corpus = String::from(wiki_full_text);
    for doc in &scraped_docs {
        full_corpus.push('\n');
        full_corpus.push_str(&doc.title);
        full_corpus.push('\n');
        full_corpus.push_str(&doc.text);
    }
    entity_pivots = leadfinder_providers::extract_entity_pivots(&full_corpus, &name);
    let eik_numbers = leadfinder_providers::extract_eik_numbers(&full_corpus);

    let delta_summary = if let Some(ref prev) = existing_person {
        if prev.researched_at.is_some() {
            let mut deltas = Vec::new();
            if newly_recorded_events > 0 {
                deltas.push(format!("{} нови медийни публикации", newly_recorded_events));
            }
            if !sanctions_matches.is_empty() {
                deltas.push(format!(
                    "{} съвпадения в санкционни/PEP регистри",
                    sanctions_matches.len()
                ));
            }
            if !entity_pivots.is_empty() {
                deltas.push(format!(
                    "{} установени търговски дружества/организации ({})",
                    entity_pivots.len(),
                    entity_pivots
                        .iter()
                        .take(3)
                        .map(|p| p.entity_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if !temporal_audit.historical_snapshots.is_empty() {
                deltas.push(format!(
                    "{} архивни уеб копия в Wayback Machine",
                    temporal_audit.historical_snapshots.len()
                ));
            }
            if deltas.is_empty() {
                "Няма установени съществени промени спрямо предходната проверка.".to_string()
            } else {
                format!("Промени спрямо предходната проверка: {}", deltas.join("; "))
            }
        } else {
            format!(
                "Първоначална справка: проверени {} източника, {} дружества/организации и {} архивни записа.",
                scraped_docs.len(),
                entity_pivots.len(),
                temporal_audit.historical_snapshots.len()
            )
        }
    } else {
        format!(
            "Първоначална справка: проверени {} източника и {} свързани организации.",
            scraped_docs.len(),
            entity_pivots.len()
        )
    };

    let wiki_sections = parse_wiki_sections(wiki_full_text);
    let headings = chapter_headings_for_type(&entity_type);

    let fin_extracted = extract_matching_factual_passages(
        &scraped_docs,
        &["fin", "pivot"],
        wiki_full_text,
        &[
            "еик", "булстат", "оод", "еоод", "ад", "еад", "дззд", "търговски регистър",
            "управител", "съдружник", "дял", "капитал", "собственост", "имот", "декларац",
            "кпконпи", "сметна палата", "поръчк", "договор", "лв", "лева", "евро", "долар", "$",
            "млн", "млрд", "милион", "милиард", "финанс", "богатств", "актив", "приход", "печалб",
            "доход", "компани", "корпорац", "организейшън", "холдинг", "консорциум", "сделк",
            "инвеститор", "изпълнител", "акци",
        ],
        22,
    );
    let net_extracted = extract_matching_factual_passages(
        &scraped_docs,
        &["net", "pivot"],
        wiki_full_text,
        &[
            "съдружник", "партньор", "свързан", "кръг", "коалиция", "партия", "управителен",
            "съвет", "директор", "основател", "назнач", "предложен", "екип", "консорциум",
            "семейств", "вицепрезидент", "съветник", "администрац",
        ],
        16,
    );
    let legal_extracted = extract_matching_factual_passages(
        &scraped_docs,
        &["legal", "gov"],
        wiki_full_text,
        &[
            "съд", "вас", "вкс", "сес", "дело", "решение", "присъд", "осъден", "оправдан",
            "обвинен", "импийчмънт", "иск", "оспор", "кпконпи", "кпк", "несъвместимост",
            "прокуратур", "прокурор", "разследв", "проверка", "санкц", "магнитски", "ofac",
            "глоба", "акт", "днск", "кзк", "отстранен", "обжалв",
        ],
        22,
    );

    let mut b1_sources = Vec::new();
    if !wiki_sections.is_empty() {
        for (stitle, sbody) in wiki_sections.iter().take(8) {
            b1_sources.push(format!("### Уикипедия: {}\n{}", stitle, sbody.chars().take(3200).collect::<String>()));
        }
    }
    if !eik_numbers.is_empty() {
        b1_sources.push(format!("Открити ЕИК / БУЛСТАТ номера в източниците: {}", eik_numbers.join(", ")));
    }
    if !entity_pivots.is_empty() {
        let piv_lines: Vec<String> = entity_pivots
            .iter()
            .map(|p| {
                format!(
                    "- {} ({}) — {}",
                    p.entity_name,
                    p.entity_kind,
                    p.registry_status.as_deref().unwrap_or("открити данни"),
                )
            })
            .collect();
        b1_sources.push(format!("Установени търговски дружества и организации:\n{}", piv_lines.join("\n")));
    }
    if !fin_extracted.is_empty() {
        let lines: Vec<String> = fin_extracted
            .iter()
            .map(|fact| format!("- {}", fact))
            .collect();
        b1_sources.push(format!("Извлечени конкретни финансови, фирмени и имотни факти:\n{}", lines.join("\n")));
    }
    if !net_extracted.is_empty() {
        let lines: Vec<String> = net_extracted
            .iter()
            .map(|fact| format!("- {}", fact))
            .collect();
        b1_sources.push(format!("Извлечени факти за свързани лица и организации:\n{}", lines.join("\n")));
    }
    for doc in scraped_docs
        .iter()
        .filter(|d| d.bucket == "fin" || d.bucket == "net" || d.bucket == "pivot")
        .take(10)
    {
        b1_sources.push(format!(
            "Документ [{}] ({}):\n{}",
            doc.title,
            doc.url,
            doc.text.chars().take(1800).collect::<String>()
        ));
    }
    let b1_context = b1_sources.join("\n\n").chars().take(24000).collect::<String>();

    let mut b2_sources = Vec::new();
    b2_sources.push(sanctions_context_block.clone());
    if wiki_sections.len() > 1 {
        for (stitle, sbody) in wiki_sections.iter().skip(1) {
            b2_sources.push(format!("### Уикипедия: {}\n{}", stitle, sbody.chars().take(2800).collect::<String>()));
        }
    }
    if !legal_extracted.is_empty() {
        let lines: Vec<String> = legal_extracted
            .iter()
            .map(|fact| format!("- {}", fact))
            .collect();
        b2_sources.push(format!("Извлечени конкретни правни, регулаторни и съдебни факти:\n{}", lines.join("\n")));
    }
    for doc in scraped_docs
        .iter()
        .filter(|d| d.bucket == "gov" || d.bucket == "legal" || d.bucket == "media")
        .take(12)
    {
        b2_sources.push(format!(
            "Документ [{}] ({}):\n{}",
            doc.title,
            doc.url,
            doc.text.chars().take(1800).collect::<String>()
        ));
    }
    let b2_context = b2_sources.join("\n\n").chars().take(24000).collect::<String>();

    let mut b3_sources = Vec::new();
    b3_sources.push(aegis_summary_str.clone());
    b3_sources.push(archive_context_block.clone());
    for (idx, doc) in scraped_docs.iter().enumerate() {
        if !doc.url.is_empty() {
            b3_sources.push(format!("{}. {} — {}", idx + 1, doc.title, doc.url));
        }
    }
    let b3_context = b3_sources.join("\n\n").chars().take(18000).collect::<String>();

    ctx.update_active_task(
        &task_id,
        "Съставяне на аналитичната справка (Глави 1–9)",
        78,
    )
    .await;

    let runner =
        leadfinder_providers::LlmChainRunner::from_role("osint_dossier", &ctx.paths, &cfg, &ctx.db)
            .await;

    let chapter_block_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "chapter_markdown": { "type": "string" }
        },
        "required": ["chapter_markdown"]
    });

    let anti_boilerplate_rules = "СТРИКТНИ ПРАВИЛА ЗА СТИЛ И ТОЧНОСТ:\n\
        1. Пиши в строг, неутрален, документален стил (като официален правен, одитен или журналистически доклад).\n\
        2. АБСОЛЮТНО ЗАБРАНЕНО е използването на шпионски/разузнавателни клишета ('разузнавателно досие', 'обект на разследване', 'оперативен профил', 'радар', 'картографиране').\n\
        3. АБСОЛЮТНО ЗАБРАНЕНО е писането на общи учебникарски дефиниции какво представляват Търговският регистър, КПКОНПИ или Сметната палата (напр. НЕ пиши 'При автоматизирания мониторинг се проверяват преки и непреки участия...').\n\
        4. Посочвай САМО конкретни факти от подадените данни: точни имена на дружества (ООД/ЕООД/АД/ДЗЗД), ЕИК номера, дялове, конкретни дати, суми, номера/страни по дела (ВАС, ВКС, СЕС), актове на КПКОНПИ/БНБ/ДНСК/АОП и точни URL източници.\n\
        5. Ако за дадена подточка липсват публични вписвания в подадените източници, запиши това ясно и кратко в 1-2 конкретни изречения (кои източници са проверени и че не са установени данни), без общи фрази.\n\
        6. Използвай подзаглавия ### (напр. ### 2.1. ...) за прегледно съдържание.";

    let b1_system = format!(
        "Ти си правен и финансов анализатор. Състави детайлен документален текст на български език за Глави 1, 2 и 3 в полето 'chapter_markdown':\n\
        # {}\n\
        # {}\n\
        # {}\n\n\
        {}\nВърни валиден JSON.",
        headings[0], headings[1], headings[2], anti_boilerplate_rules
    );
    let b1_req = leadfinder_core::LlmJsonRequest {
        system_prompt: b1_system,
        user_prompt: format!(
            "Обект: {} (Категория: {})\n\nСвързани локални видеа:\n{}\n\nСъбрани документални и регистърни данни:\n{}",
            name,
            entity_type_bg_label(&entity_type),
            vid_context_block,
            b1_context
        ),
        json_schema: Some(chapter_block_schema.clone()),
        max_tokens: 4096,
        temperature: 0.1,
    };

    let b2_system = format!(
        "Ти си правен и институционален анализатор. Състави детайлен документален текст на български език за Глави 4, 5 и 6 в полето 'chapter_markdown':\n\
        # {}\n\
        # {}\n\
        # {}\n\n\
        {}\nВърни валиден JSON.",
        headings[3], headings[4], headings[5], anti_boilerplate_rules
    );
    let b2_req = leadfinder_core::LlmJsonRequest {
        system_prompt: b2_system,
        user_prompt: format!(
            "Обект: {} (Категория: {})\n\nСанкционни регистри:\n{}\n\nСвързани локални видеа:\n{}\n\nСъбрани правни, институционални и медийни данни:\n{}",
            name,
            entity_type_bg_label(&entity_type),
            sanctions_context_block,
            vid_context_block,
            b2_context
        ),
        json_schema: Some(chapter_block_schema.clone()),
        max_tokens: 4096,
        temperature: 0.1,
    };

    let b3_system = format!(
        "Ти си анализатор по проверка на източници и архиви. Състави детайлен документален текст на български език за Глави 7, 8 и 9 в полето 'chapter_markdown':\n\
        # {}\n\
        # {}\n\
        # {}\n\n\
        {}\nВ Глава 9 включи пълен номериран списък на проверените URL източници. Върни валиден JSON.",
        headings[6], headings[7], headings[8], anti_boilerplate_rules
    );
    let b3_req = leadfinder_core::LlmJsonRequest {
        system_prompt: b3_system,
        user_prompt: format!(
            "Обект: {} (Категория: {})\n\nПроверка на аудио-визуални записи:\n{}\n\nСанкционни регистри:\n{}\n\nСвързани локални видеа:\n{}\n\nАрхивни данни и проверени източници:\n{}",
            name,
            entity_type_bg_label(&entity_type),
            aegis_summary_str,
            sanctions_context_block,
            vid_context_block,
            b3_context
        ),
        json_schema: Some(chapter_block_schema),
        max_tokens: 4096,
        temperature: 0.1,
    };

    let extract_md = |res: Result<
        Result<leadfinder_providers::ValidatedLlmResult, leadfinder_core::ProviderError>,
        tokio::time::error::Elapsed,
    >|
     -> String {
        res.ok()
            .and_then(|inner| inner.ok())
            .and_then(|val| {
                val.response
                    .parsed_json
                    .get("chapter_markdown")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default()
    };

    let primary_prov = runner.primary_provider_name();
    let (block1_md, block2_md, block3_md) = if primary_prov == "heuristics" {
        (String::new(), String::new(), String::new())
    } else if primary_prov == "local" {
        let step_timeout = std::time::Duration::from_secs(65);
        let r1 = tokio::time::timeout(step_timeout, runner.complete_validated(&b1_req, &ctx.db)).await;
        let r2 = tokio::time::timeout(step_timeout, runner.complete_validated(&b2_req, &ctx.db)).await;
        let r3 = tokio::time::timeout(step_timeout, runner.complete_validated(&b3_req, &ctx.db)).await;
        (extract_md(r1), extract_md(r2), extract_md(r3))
    } else {
        let llm_timeout = std::time::Duration::from_secs(75);
        let (r1, r2, r3) = tokio::join!(
            tokio::time::timeout(llm_timeout, runner.complete_validated(&b1_req, &ctx.db)),
            tokio::time::timeout(llm_timeout, runner.complete_validated(&b2_req, &ctx.db)),
            tokio::time::timeout(llm_timeout, runner.complete_validated(&b3_req, &ctx.db)),
        );
        (extract_md(r1), extract_md(r2), extract_md(r3))
    };

    let has_generic_boilerplate = |txt: &str| -> bool {
        let low = txt.to_lowercase();
        low.contains("при автоматизирания мониторинг на откритите регистри")
            || low.contains("проверка в търговския регистър за преки и непреки участия")
            || low.contains("отразява свързаност с институционалния или частния сектор")
            || low.contains("анализират задължителните годишни декларации за недвижими имоти")
            || low.contains("наличие на индивидуална собственост, съдружия или участия в надзорни")
    };

    let block1_raw = if block1_md.split_whitespace().count() >= 180 && !has_generic_boilerplate(&block1_md) {
        block1_md.trim().to_string()
    } else {
        generate_fallback_block1(
            &name,
            &entity_type,
            &temporal_audit,
            &entity_pivots,
            &eik_numbers,
            &scraped_docs,
        )
    };
    let block1_clean = ensure_concrete_facts_preserved_block1(
        block1_raw,
        &entity_pivots,
        &eik_numbers,
        &fin_extracted,
        &net_extracted,
    );

    let block2_raw = if block2_md.split_whitespace().count() >= 180 && !has_generic_boilerplate(&block2_md) {
        block2_md.trim().to_string()
    } else {
        generate_fallback_block2(
            &name,
            &entity_type,
            &temporal_audit,
            &sanctions_matches,
            &scraped_docs,
        )
    };
    let block2_clean = ensure_concrete_facts_preserved_block2(
        block2_raw,
        &sanctions_matches,
        &legal_extracted,
    );

    let block3_clean = if block3_md.split_whitespace().count() >= 140 && !has_generic_boilerplate(&block3_md) {
        block3_md.trim().to_string()
    } else {
        generate_fallback_block3(
            &name,
            &entity_type,
            &aegis_summary_str,
            &temporal_audit,
            &video_contexts,
            &scraped_docs,
        )
    };

    let mut dossier_parts: Vec<String> = Vec::new();
    dossier_parts.push(format!(
        "# АНАЛИТИЧНА СПРАВКА: {}\n\n**Обект**: {} | **Категория**: {} | **Проверени източници**: {}\n\n> [!NOTE]\n> **{}**\n",
        name,
        name,
        entity_type_bg_label(&entity_type),
        scraped_docs.len(),
        delta_summary
    ));
    dossier_parts.push(block1_clean);
    dossier_parts.push(block2_clean);
    dossier_parts.push(block3_clean);

    let report_md = dossier_parts.join("\n\n---\n\n");

    ctx.update_active_task(&task_id, "Извличане на обобщени полета и запис", 92)
        .await;

    let meta_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "role": { "type": "string" },
            "summary": { "type": "string" },
            "public_figure": { "type": "boolean" },
            "category": { "type": "string" },
            "tags": { "type": "array", "items": { "type": "string" } },
            "profile": {
                "type": "object",
                "properties": {
                    "identity": { "type": "string" },
                    "contact_locations": { "type": "string" },
                    "network_associates": { "type": "string" },
                    "education_career": { "type": "string" },
                    "financial_assets": { "type": "string" },
                    "legal_regulatory": { "type": "string" },
                    "digital_media_presence": { "type": "string" },
                    "affiliations_beliefs": { "type": "string" },
                    "psychographic_behavioral": { "type": "string" }
                }
            }
        },
        "required": ["role", "summary", "public_figure", "tags"]
    });

    let meta_req = leadfinder_core::LlmJsonRequest {
        system_prompt: "Извлечи структурирани данни на български език от подадената аналитична справка. Използвай само конкретни факти (имена на дружества, ЕИК, длъжности, решения, дати) в полетата на 'profile', без общи фрази или шпионска терминология. Върни валиден JSON.".to_string(),
        user_prompt: format!("Обект: {} (Тип: {})\n\nСправка:\n{}", name, entity_type, report_md.chars().take(10000).collect::<String>()),
        json_schema: Some(meta_schema),
        max_tokens: 2048,
        temperature: 0.0,
    };

    let mut res_role = None;
    let mut res_summary = None;
    let mut res_pub = false;
    let mut res_tags: Vec<String> = Vec::new();
    let mut res_category: Option<String> = existing_cat;
    let res_entity_type = entity_type.clone();
    let mut profile_obj = serde_json::Value::Null;

    if primary_prov != "heuristics" {
        if let Ok(Ok(val)) = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            runner.complete_validated(&meta_req, &ctx.db),
        )
        .await
        {
            res_role = val
                .response
                .parsed_json
                .get("role")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            res_summary = val
                .response
                .parsed_json
                .get("summary")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            res_pub = val
                .response
                .parsed_json
                .get("public_figure")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if let Some(cat) = val
                .response
                .parsed_json
                .get("category")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                res_category = Some(cat.to_string());
            }
            if let Some(arr) = val.response.parsed_json.get("tags").and_then(|v| v.as_array()) {
                for item in arr {
                    if let Some(t) = item.as_str() {
                        let clean = t.trim();
                        if !clean.is_empty() && !res_tags.iter().any(|x| x.eq_ignore_ascii_case(clean)) {
                            res_tags.push(clean.to_string());
                        }
                    }
                }
            }
            if let Some(p) = val.response.parsed_json.get("profile") {
                profile_obj = p.clone();
            }
        }
    }

    let fin_concrete_summary = if !fin_extracted.is_empty() {
        fin_extracted
            .iter()
            .take(3)
            .map(|f| f.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    } else if !entity_pivots.is_empty() {
        entity_pivots
            .iter()
            .map(|p| {
                if let Some(ref st) = p.registry_status {
                    format!("{} ({}, {})", p.entity_name, p.entity_kind, st)
                } else {
                    format!("{} ({})", p.entity_name, p.entity_kind)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        "Не са установени преки дружествени вписвания в проверените публични източници.".to_string()
    };

    let leg_concrete_summary = if !legal_extracted.is_empty() {
        legal_extracted
            .iter()
            .take(3)
            .map(|f| f.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        sanctions_context_block.clone()
    };

    let net_concrete_summary = if !net_extracted.is_empty() {
        net_extracted
            .iter()
            .take(3)
            .map(|f| f.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    } else if !entity_pivots.is_empty() {
        entity_pivots
            .iter()
            .map(|p| format!("{} ({})", p.entity_name, p.entity_kind))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        "Виж Глава 3 от пълния доклад.".to_string()
    };

    if profile_obj.is_null() || !profile_obj.is_object() {
        let bio_intro = wiki_sections
            .first()
            .map(|(_, b)| b.chars().take(450).collect::<String>())
            .unwrap_or_else(|| format!("{} ({})", name, entity_type_bg_label(&entity_type)));
        profile_obj = serde_json::json!({
            "identity": bio_intro,
            "financial_assets": fin_concrete_summary,
            "network_associates": net_concrete_summary,
            "legal_regulatory": leg_concrete_summary,
            "digital_media_presence": format!("Проверени {} публични източника и {} медийни публикации.", scraped_docs.len(), live_events.len()),
        });
    } else if let Some(map) = profile_obj.as_object_mut() {
        let needs_fin = map
            .get("financial_assets")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().chars().count() < 25 || has_generic_boilerplate(s))
            .unwrap_or(true);
        if needs_fin {
            map.insert(
                "financial_assets".to_string(),
                serde_json::Value::String(fin_concrete_summary),
            );
        }
        let needs_net = map
            .get("network_associates")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().chars().count() < 25 || has_generic_boilerplate(s))
            .unwrap_or(true);
        if needs_net {
            map.insert(
                "network_associates".to_string(),
                serde_json::Value::String(net_concrete_summary),
            );
        }
        let needs_leg = map
            .get("legal_regulatory")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().chars().count() < 25 || has_generic_boilerplate(s))
            .unwrap_or(true);
        if needs_leg {
            map.insert(
                "legal_regulatory".to_string(),
                serde_json::Value::String(leg_concrete_summary),
            );
        }
    }

    if res_summary.is_none() {
        if let Some((_, intro)) = wiki_sections.first() {
            let short: String = intro.chars().take(340).collect();
            res_summary = Some(short);
        } else {
            res_summary = Some(format!(
                "Аналитична справка за {} на базата на {} публични източника, регистри и архивни записи.",
                name,
                scraped_docs.len()
            ));
        }
    }
    if res_role.is_none() {
        res_role = Some(entity_type_bg_label(&entity_type).to_string());
    }
    if res_tags.is_empty() {
        res_tags = vec![entity_type_bg_label(&entity_type).to_string()];
        if !entity_pivots.is_empty() {
            res_tags.push("Търговски регистър".to_string());
        }
    }

    let full_obj = serde_json::json!({
        "report_markdown": report_md,
        "profile": profile_obj,
        "delta_summary": delta_summary,
        "sanctions_matches": sanctions_matches,
        "entity_pivots": entity_pivots,
        "temporal_audit": temporal_audit,
    });
    let res_detailed_json = Some(serde_json::to_string(&full_obj).unwrap_or_default());

    let osint_dir = std::path::PathBuf::from("data/osint");
    let _ = std::fs::create_dir_all(&osint_dir);
    let safe_name: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let report_filename = format!("{}_{}_dossier.md", id, safe_name);
    let report_path = osint_dir.join(&report_filename);
    let _ = std::fs::write(&report_path, &report_md);

    let _ = ctx
        .db
        .update_person_research_data(
            id,
            res_role.as_deref(),
            res_summary.as_deref(),
            &res_tags,
            res_detailed_json.as_deref(),
            res_pub,
            res_category.as_deref(),
            Some(&res_entity_type),
        )
        .await;
    let (total_claims, contradicted, supported) =
        ctx.db.get_person_claims_stats(id).await.unwrap_or((0, 0, 0));

    Json(serde_json::json!({
        "id": id,
        "canonical_name": name,
        "role": res_role,
        "summary": res_summary,
        "public_figure": res_pub,
        "category": res_category,
        "entity_type": res_entity_type,
        "tags": res_tags,
        "detailed_profile_json": res_detailed_json,
        "dossier_report_markdown": report_md,
        "report_file_path": format!("data/osint/{}", report_filename),
        "total_claims": total_claims,
        "contradicted_count": contradicted,
        "supported_count": supported,
        "delta_summary": delta_summary,
        "sanctions_matches": sanctions_matches,
        "entity_pivots": entity_pivots,
        "temporal_audit": temporal_audit
    }))
    .into_response()
}

fn ensure_concrete_facts_preserved_block1(
    base_md: String,
    entity_pivots: &[leadfinder_core::EntityPivotRef],
    eik_numbers: &[String],
    fin_extracted: &[String],
    net_extracted: &[String],
) -> String {
    let low = base_md.to_lowercase();
    let mut missing_lines: Vec<String> = Vec::new();

    let missing_eiks: Vec<&str> = eik_numbers
        .iter()
        .map(|e| e.as_str())
        .filter(|e| !base_md.contains(*e))
        .collect();
    if !missing_eiks.is_empty() {
        missing_lines.push(format!(
            "- **Установени ЕИК / БУЛСТАТ номера в проверените документи**: {}",
            missing_eiks.join(", ")
        ));
    }

    for piv in entity_pivots.iter().take(12) {
        if !low.contains(&piv.entity_name.to_lowercase()) {
            let status_note = piv
                .registry_status
                .as_deref()
                .unwrap_or("споменато в публични източници");
            missing_lines.push(format!(
                "- **Свързано дружество / организация**: **{}** ({}) — {}",
                piv.entity_name, piv.entity_kind, status_note
            ));
        }
    }

    for fact in fin_extracted.iter().take(8) {
        let probe: String = fact.chars().take(42).collect::<String>().to_lowercase();
        if !probe.is_empty() && !low.contains(&probe) {
            missing_lines.push(format!("- {}", fact));
        }
    }

    for fact in net_extracted.iter().take(5) {
        let probe: String = fact.chars().take(42).collect::<String>().to_lowercase();
        if !probe.is_empty() && !low.contains(&probe) && !missing_lines.iter().any(|l| l.to_lowercase().contains(&probe)) {
            missing_lines.push(format!("- {}", fact));
        }
    }

    if missing_lines.is_empty() {
        base_md
    } else {
        format!(
            "{}\n\n### Потвърдени конкретни вписвания и факти от проверените източници\n{}",
            base_md.trim_end(),
            missing_lines.join("\n")
        )
    }
}

fn ensure_concrete_facts_preserved_block2(
    base_md: String,
    sanctions_matches: &[leadfinder_core::SanctionsMatch],
    legal_extracted: &[String],
) -> String {
    let low = base_md.to_lowercase();
    let mut missing_lines: Vec<String> = Vec::new();

    for sm in sanctions_matches.iter().take(8) {
        if !low.contains(&sm.dataset.to_lowercase()) && !low.contains(&sm.caption.to_lowercase()) {
            missing_lines.push(format!(
                "- **Санкционен / PEP регистър [{}]**: **{}** (съвпадение {:.0}%)",
                sm.dataset,
                sm.caption,
                sm.match_score * 100.0
            ));
        }
    }

    for fact in legal_extracted.iter().take(8) {
        let probe: String = fact.chars().take(42).collect::<String>().to_lowercase();
        if !probe.is_empty() && !low.contains(&probe) {
            missing_lines.push(format!("- {}", fact));
        }
    }

    if missing_lines.is_empty() {
        base_md
    } else {
        format!(
            "{}\n\n### Потвърдени конкретни съдебни, регулаторни и санкционни данни от източниците\n{}",
            base_md.trim_end(),
            missing_lines.join("\n")
        )
    }
}

fn parse_wiki_sections(raw_text: &str) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let mut current_title = "Въведение и основен профил".to_string();
    let mut current_lines: Vec<String> = Vec::new();

    for line in raw_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("==") && trimmed.ends_with("==") {
            let body = current_lines.join("\n").trim().to_string();
            if !body.is_empty() {
                sections.push((current_title, body));
            }
            current_lines.clear();
            current_title = trimmed.trim_matches('=').trim().to_string();
        } else {
            current_lines.push(line.to_string());
        }
    }
    let body = current_lines.join("\n").trim().to_string();
    if !body.is_empty() {
        sections.push((current_title, body));
    }
    sections
}

#[derive(Debug, Clone)]
struct ScrapedOsintDoc {
    bucket: String,
    title: String,
    url: String,
    text: String,
}

fn entity_type_bg_label(entity_type: &str) -> &'static str {
    match entity_type {
        "institution" => "Институция",
        "channel" => "Медия / Канал",
        "country" => "Държава",
        "group" => "Организация / Партия",
        "event" => "Събитие",
        "infrastructure" => "Инфраструктурен обект",
        "building" => "Сграда / Обект",
        _ => "Личност",
    }
}

fn chapter_headings_for_type(entity_type: &str) -> [&'static str; 9] {
    match entity_type {
        "infrastructure" | "building" => [
            "ГЛАВА 1: ОПИСАНИЕ НА ОБЕКТА, МЕСТОПОЛОЖЕНИЕ И ХРОНОЛОГИЯ",
            "ГЛАВА 2: ФИНАНСИРАНЕ, ОБЩЕСТВЕНИ ПОРЪЧКИ, ДОГОВОРИ И СТОЙНОСТ",
            "ГЛАВА 3: ВЪЗЛОЖИТЕЛИ, ИЗПЪЛНИТЕЛИ, КОНСОРЦИУМИ И НАДЗОР",
            "ГЛАВА 4: СТРОИТЕЛНИ ЕТАПИ, РАЗРЕШИТЕЛНИ РЕЖИМИ И ЕКСПЛОАТАЦИЯ",
            "ГЛАВА 5: КОНТРОЛНИ ПРОВЕРКИ, СЪДЕБНИ СПОРОВЕ И САНКЦИИ",
            "ГЛАВА 6: ЖУРНАЛИСТИЧЕСКИ РАЗСЛЕДВАНИЯ, ИНЦИДЕНТИ И ОБЩЕСТВЕН ОТЗВУК",
            "ГЛАВА 7: ПРОВЕРКА ЗА МАНИПУЛИРАНИ ЗАПИСИ И ДЕЗИНФОРМАЦИЯ",
            "ГЛАВА 8: АРХИВНИ УЕБ ВЕРСИИ И ИСТОРИЯ НА ПУБЛИКАЦИИТЕ",
            "ГЛАВА 9: ОБОБЩЕНА ОЦЕНКА И СПИСЪК НА ПРОВЕРЕНИТЕ ИЗТОЧНИЦИ",
        ],
        "institution" | "group" => [
            "ГЛАВА 1: ПРАВЕН СТАТУТ, СТРУКТУРА, РЪКОВОДСТВО И ИСТОРИЯ",
            "ГЛАВА 2: БЮДЖЕТ, ФИНАНСИРАНЕ, ДОГОВОРИ И ИМУЩЕСТВЕНИ ДАННИ",
            "ГЛАВА 3: СВЪРЗАНИ ЛИЦА, ОРГАНИЗАЦИИ И ПАРТНЬОРСКИ СТРУКТУРИ",
            "ГЛАВА 4: УПРАВЛЕНСКА ДЕЙНОСТ, ПУБЛИЧНИ ПРОГРАМИ И РЕШЕНИЯ",
            "ГЛАВА 5: СЪДЕБНИ ПРОИЗВОДСТВА, ОДИТИ, ПРОВЕРКИ И САНКЦИИ",
            "ГЛАВА 6: МЕДИЙНИ ПУБЛИКАЦИИ, РАЗСЛЕДВАНИЯ И ПУБЛИЧНИ ПОЗИЦИИ",
            "ГЛАВА 7: ПРОВЕРКА ЗА МАНИПУЛИРАНИ ЗАПИСИ И ДЕЗИНФОРМАЦИЯ",
            "ГЛАВА 8: АРХИВНИ УЕБ ВЕРСИИ И ИСТОРИЯ НА ПУБЛИКАЦИИТЕ",
            "ГЛАВА 9: ОБОБЩЕНА ОЦЕНКА И СПИСЪК НА ПРОВЕРЕНИТЕ ИЗТОЧНИЦИ",
        ],
        _ => [
            "ГЛАВА 1: БИОГРАФИЧНИ ДАННИ, ОБРАЗОВАНИЕ И ПРОФЕСИОНАЛЕН ПЪТ",
            "ГЛАВА 2: ТЪРГОВСКИ РЕГИСТЪР, ФИРМЕНИ УЧАСТИЯ И ИМУЩЕСТВЕНИ ДЕКЛАРАЦИИ",
            "ГЛАВА 3: СВЪРЗАНИ ЛИЦА, СЪДРУЖНИЦИ И ОРГАНИЗАЦИОННА СРЕДА",
            "ГЛАВА 4: ПУБЛИЧНИ ДЛЪЖНОСТИ, УПРАВЛЕНСКА ДЕЙНОСТ И ОБЩЕСТВЕНИ ПОРЪЧКИ",
            "ГЛАВА 5: СЪДЕБНИ ПРОИЗВОДСТВА, АКТОВЕ НА КПКОНПИ И САНКЦИОННИ СПИСЪЦИ",
            "ГЛАВА 6: МЕДИЙНИ РАЗСЛЕДВАНИЯ, ПУБЛИЧНИ ПОЗИЦИИ И ПОЛЕМИКИ",
            "ГЛАВА 7: ПРОВЕРКА ЗА СИНТЕТИЧНО СЪДЪРЖАНИЕ И МАНИПУЛИРАНИ ЗАПИСИ",
            "ГЛАВА 8: АРХИВНИ УЕБ ВЕРСИИ И ИСТОРИЯ НА ПУБЛИКАЦИИТЕ",
            "ГЛАВА 9: ОБОБЩЕНА ОЦЕНКА И СПИСЪК НА ПРОВЕРЕНИТЕ ИЗТОЧНИЦИ",
        ],
    }
}

fn split_factual_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.lines() {
        let p = para.trim();
        if p.chars().count() < 35 {
            continue;
        }
        let mut cur = String::new();
        let chars: Vec<char> = p.chars().collect();
        for i in 0..chars.len() {
            let c = chars[i];
            cur.push(c);
            let is_end = (c == '.' || c == '!' || c == '?')
                && (i + 1 == chars.len() || chars[i + 1].is_whitespace());
            if is_end {
                let prev_word: String = cur
                    .trim_end_matches(|ch: char| ch == '.' || ch == '!' || ch == '?')
                    .split_whitespace()
                    .last()
                    .unwrap_or("")
                    .to_lowercase();
                let abbrev = matches!(
                    prev_word.as_str(),
                    "г" | "гр" | "с" | "ул" | "бул" | "ал" | "чл" | "т" | "параграф"
                        | "проф" | "доц" | "д-р" | "инж" | "адв" | "ген" | "млн" | "млрд" | "хил" | "лв"
                );
                if !abbrev && cur.chars().count() >= 40 {
                    out.push(cur.trim().to_string());
                    cur.clear();
                }
            }
        }
        if cur.trim().chars().count() >= 40 {
            out.push(cur.trim().to_string());
        }
    }
    out
}

fn extract_matching_factual_passages(
    docs: &[ScrapedOsintDoc],
    preferred_buckets: &[&str],
    wiki_text: &str,
    keywords: &[&str],
    max_items: usize,
) -> Vec<String> {
    let mut scored: Vec<(usize, String)> = Vec::new();
    let mut seen_prefixes: Vec<String> = Vec::new();

    let mut ordered_docs: Vec<&ScrapedOsintDoc> = docs
        .iter()
        .filter(|d| preferred_buckets.contains(&d.bucket.as_str()))
        .collect();
    for d in docs {
        if !preferred_buckets.contains(&d.bucket.as_str()) {
            ordered_docs.push(d);
        }
    }

    for doc in ordered_docs {
        let bucket_bonus = if preferred_buckets.contains(&doc.bucket.as_str()) { 3 } else { 0 };
        for sent in split_factual_sentences(&doc.text) {
            let low = sent.to_lowercase();
            if low.contains("бисквитки") || low.contains("cookies") || low.contains("всички права запазени") || low.contains("абонирайте се") {
                continue;
            }
            let kw_hits = keywords.iter().filter(|kw| low.contains(**kw)).count();
            if kw_hits == 0 {
                continue;
            }
            let has_digit = sent.chars().any(|c| c.is_ascii_digit());
            let has_quote = sent.contains('„') || sent.contains('"');
            let score = kw_hits * 2 + bucket_bonus + if has_digit { 2 } else { 0 } + if has_quote { 1 } else { 0 };
            let prefix_key: String = low.chars().take(55).collect();
            if seen_prefixes.iter().any(|p| p == &prefix_key) {
                continue;
            }
            seen_prefixes.push(prefix_key);
            let clean_title = if doc.title.trim().is_empty() { "Публичен източник" } else { doc.title.trim() };
            let formatted = if doc.url.starts_with("http") {
                format!("{} ([{}]({}))", sent, clean_title, doc.url)
            } else {
                format!("{} ({})", sent, clean_title)
            };
            scored.push((score, formatted));
        }
    }

    if scored.len() < max_items && !wiki_text.is_empty() {
        for sent in split_factual_sentences(wiki_text) {
            let low = sent.to_lowercase();
            let kw_hits = keywords.iter().filter(|kw| low.contains(**kw)).count();
            if kw_hits == 0 {
                continue;
            }
            let prefix_key: String = low.chars().take(55).collect();
            if seen_prefixes.iter().any(|p| p == &prefix_key) {
                continue;
            }
            seen_prefixes.push(prefix_key);
            let has_digit = sent.chars().any(|c| c.is_ascii_digit());
            let score = kw_hits * 2 + if has_digit { 2 } else { 0 };
            scored.push((score, format!("{} (Уикипедия)", sent)));
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(max_items).map(|(_, s)| s).collect()
}

fn generate_fallback_block1(
    name: &str,
    entity_type: &str,
    temporal_audit: &leadfinder_providers::AegisTemporalAudit,
    entity_pivots: &[leadfinder_core::EntityPivotRef],
    eik_numbers: &[String],
    docs: &[ScrapedOsintDoc],
) -> String {
    let headings = chapter_headings_for_type(entity_type);
    let wiki_full = temporal_audit.wiki_bio.as_ref().map(|w| w.extract.as_str()).unwrap_or_default();
    let mut out = String::new();

    out.push_str(&format!("# {}\n\n", headings[0]));
    if let Some(ref wb) = temporal_audit.wiki_bio {
        let sections = parse_wiki_sections(&wb.extract);
        let mut added = 0usize;
        for (stitle, sbody) in &sections {
            let slow = stitle.to_lowercase();
            if slow.contains("въведение") || slow.contains("биография") || slow.contains("ранен")
                || slow.contains("образование") || slow.contains("кариера") || slow.contains("професионал")
                || slow.contains("история") || slow.contains("описание") || slow.contains("строителство")
            {
                out.push_str(&format!("### {}\n{}\n\n", stitle, sbody));
                added += 1;
            }
        }
        if added == 0 {
            out.push_str(&format!("### Основни данни ({})\n{}\n\n", wb.title, wb.extract.chars().take(3500).collect::<String>()));
        }
    } else {
        let bio_facts = extract_matching_factual_passages(
            docs,
            &["bio", "gov"],
            wiki_full,
            &["роден", "завършва", "университет", "образование", "кариера", "директор", "председател", "министър", "кмет", "изграден", "открит", "местоположение", "площ", "км"],
            8,
        );
        if !bio_facts.is_empty() {
            out.push_str("### 1.1. Установени хронологични и основни данни от проверените източници\n");
            for f in bio_facts {
                out.push_str(&format!("- {}\n", f));
            }
            out.push('\n');
        } else {
            out.push_str(&format!(
                "В проверените публични източници за **{}** (тип: {}) не беше открита самостоятелна биографична статия в Уикипедия; наличните сведения са отразени в тематичните раздели по-долу.\n\n",
                name,
                entity_type_bg_label(entity_type)
            ));
        }
    }

    out.push_str(&format!("# {}\n\n", headings[1]));
    let fin_keywords: &[&str] = if entity_type == "infrastructure" || entity_type == "building" {
        &["поръчка", "договор", "стойност", "бюджет", "лв", "лева", "евро", "долар", "$", "млн", "млрд", "милион", "милиард", "финансиране", "аоп", "цаис", "изпълнител", "индексация", "аванс", "сметна палата", "еик"]
    } else {
        &["оод", "еоод", "ад", "еад", "дззд", "еик", "търговски регистър", "дял", "дялове", "капитал", "съдружник", "управител", "собственик", "декларация", "кпконпи", "кпк", "имот", "влог", "лв", "лева", "евро", "долар", "$", "млн", "млрд", "милион", "милиард", "богатств", "актив", "доход", "печалб", "компани", "корпорац", "организейшън", "холдинг", "сделк", "оборот", "поръчка", "договор"]
    };
    let fin_facts = extract_matching_factual_passages(docs, &["fin", "pivot", "legal"], wiki_full, fin_keywords, 12);

    if !entity_pivots.is_empty() || !eik_numbers.is_empty() {
        out.push_str("### 2.1. Идентифицирани юридически лица, консорциуми и регистрови идентификатори\n");
        for p in entity_pivots {
            out.push_str(&format!(
                "- **{}** — вид: {}, бележка: {}\n",
                p.entity_name,
                p.entity_kind,
                p.registry_status.as_deref().unwrap_or("открито в източниците")
            ));
        }
        if !eik_numbers.is_empty() {
            out.push_str(&format!("- **Открити ЕИК / БУЛСТАТ номера в документите**: {}\n", eik_numbers.join(", ")));
        }
        out.push('\n');
    }

    out.push_str("### 2.2. Конкретни констатации за участия, имущество, договори и финансиране\n");
    if !fin_facts.is_empty() {
        for f in &fin_facts {
            out.push_str(&format!("- {}\n", f));
        }
        out.push('\n');
    } else {
        out.push_str(&format!(
            "В извлечените публични публикации и регистрови извадки не се съдържат данни за преки дружествени дялове, вписани ЕИК номера или имуществени несъответствия за **{}**.\n\n",
            name
        ));
    }

    out.push_str(&format!("# {}\n\n", headings[2]));
    let net_keywords: &[&str] = if entity_type == "infrastructure" || entity_type == "building" {
        &["възложител", "изпълнител", "консорциум", "дззд", "надзор", "проектант", "архитект", "апи", "мррб", "община", "подизпълнител", "фирма", "дружество"]
    } else {
        &["съдружник", "партньор", "сдружение", "фондация", "управителен съвет", "съвет на директорите", "директор", "основател", "коалиция", "партия", "екип", "заместник", "вицепрезидент", "назначен", "предложен", "семейств", "близък", "свързан"]
    };
    let net_facts = extract_matching_factual_passages(docs, &["net", "pivot", "fin", "gov"], wiki_full, net_keywords, 10);

    out.push_str("### 3.1. Установени свързани лица, организации и партньорства\n");
    if !net_facts.is_empty() {
        for f in &net_facts {
            out.push_str(&format!("- {}\n", f));
        }
        out.push('\n');
    } else if !entity_pivots.is_empty() {
        for p in entity_pivots {
            out.push_str(&format!("- В проверените материали **{}** се споменава във връзка със субекта **{}** ({}).\n", name, p.entity_name, p.entity_kind));
        }
        out.push('\n');
    } else {
        out.push_str(&format!(
            "В проверените публични източници не са установени данни за неформални корпоративни съдружия или свързани търговски групи около **{}**.\n\n",
            name
        ));
    }

    out
}

fn generate_fallback_block2(
    name: &str,
    entity_type: &str,
    temporal_audit: &leadfinder_providers::AegisTemporalAudit,
    sanctions_matches: &[leadfinder_core::SanctionsMatch],
    docs: &[ScrapedOsintDoc],
) -> String {
    let headings = chapter_headings_for_type(entity_type);
    let wiki_full = temporal_audit.wiki_bio.as_ref().map(|w| w.extract.as_str()).unwrap_or_default();
    let mut out = String::new();

    out.push_str(&format!("# {}\n\n", headings[3]));
    let gov_keywords: &[&str] = if entity_type == "infrastructure" || entity_type == "building" {
        &["строеж", "разрешение", "акт 16", "акт 15", "днск", "мррб", "апи", "пусков", "лот", "участък", "ремонт", "реконструкция", "експлоатация", "проект"]
    } else {
        &["мандат", "правителство", "кабинет", "министър", "парламент", "народно събрание", "депутат", "бнб", "подуправител", "председател", "комисия", "кмет", "президент", "указ", "бюджет", "решение"]
    };
    let gov_facts = extract_matching_factual_passages(docs, &["gov", "bio"], wiki_full, gov_keywords, 10);
    if !gov_facts.is_empty() {
        out.push_str("### 4.1. Данни за публични правомощия, управленски решения и етапи\n");
        for f in &gov_facts {
            out.push_str(&format!("- {}\n", f));
        }
        out.push('\n');
    } else if let Some(ref wb) = temporal_audit.wiki_bio {
        let sections = parse_wiki_sections(&wb.extract);
        let mut any_sec = false;
        for (stitle, sbody) in sections.iter().skip(1).take(3) {
            out.push_str(&format!("### {}\n{}\n\n", stitle, sbody));
            any_sec = true;
        }
        if !any_sec {
            out.push_str(&format!("Няма регистрирани отделни данни за държавни мандати или строителни разрешения за **{}** в проверените източници.\n\n", name));
        }
    } else {
        out.push_str(&format!("В проверените източници не са установени допълнителни данни за държавни постове или обществени поръчки, пряко управлявани от **{}**.\n\n", name));
    }

    out.push_str(&format!("# {}\n\n", headings[4]));
    let legal_keywords: &[&str] = &[
        "съд", "вас", "вкс", "сгс", "административен съд", "сес", "люксембург", "дело", "решение",
        "присъд", "осъден", "оправдан", "импийчмънт", "иск", "оспор", "определение", "жалба",
        "прокуратура", "прокурор", "разследване", "обвинение", "обвинения", "кпконпи", "кпк",
        "несъвместимост", "конфликт на интереси", "отстраняване", "отстранен", "кзк", "сметна палата",
        "нарушение", "глоба", "санкция", "санкции", "магнитски", "ofac", "проверка", "акт",
    ];
    let legal_facts = extract_matching_factual_passages(docs, &["legal", "fin", "media"], wiki_full, legal_keywords, 12);

    out.push_str("### 5.1. Съдебни дела, производства пред регулаторни органи и проверки\n");
    if !legal_facts.is_empty() {
        for f in &legal_facts {
            out.push_str(&format!("- {}\n", f));
        }
        out.push('\n');
    } else {
        out.push_str(&format!(
            "В проверените публични източници не са открити данни за влезли в сила осъдителни присъди, висящи съдебни производства или установени нарушения от КПКОНПИ/контролни органи срещу **{}**.\n\n",
            name
        ));
    }

    out.push_str("### 5.2. Справка в международни санкционни списъци\n");
    if sanctions_matches.is_empty() {
        out.push_str("Проверени регистри: ООН, САЩ (OFAC SDN / Закон „Магнитски“), Европейски съюз, Великобритания (OFSI), Швейцария (SECO), Украйна (НАЗК) и Интерпол — **няма открити санкционни вписвания**.\n\n");
    } else {
        for sm in sanctions_matches {
            out.push_str(&format!(
                "- **[{}]** {} — съвпадение {:.0}%, програма: {}\n",
                sm.dataset,
                sm.caption,
                sm.match_score * 100.0,
                sm.program.as_deref().unwrap_or("неуточнена")
            ));
        }
        out.push('\n');
    }

    out.push_str(&format!("# {}\n\n", headings[5]));
    let media_facts = extract_matching_factual_passages(
        docs,
        &["media", "news", "legal"],
        wiki_full,
        &["разследване", "публикация", "позиция", "заяви", "коментира", "критика", "спор", "скандал", "казус", "интервю", "инцидент", "протест"],
        10,
    );
    out.push_str("### 6.1. Основни теми в медийните публикации и журналистически анализи\n");
    if !media_facts.is_empty() {
        for f in &media_facts {
            out.push_str(&format!("- {}\n", f));
        }
        out.push('\n');
    } else {
        out.push_str("Не са установени допълнителни медийни полемики извън описаните в предходните глави.\n\n");
    }

    out
}

fn generate_fallback_block3(
    name: &str,
    entity_type: &str,
    aegis_summary: &str,
    temporal_audit: &leadfinder_providers::AegisTemporalAudit,
    video_contexts: &[String],
    docs: &[ScrapedOsintDoc],
) -> String {
    let headings = chapter_headings_for_type(entity_type);
    let mut out = String::new();

    out.push_str(&format!(
        "# {}\n\n### 7.1. Резултати от аудио-визуалната проверка на свързаните видеа\n{}\n\n",
        headings[6], aegis_summary
    ));

    if !video_contexts.is_empty() {
        out.push_str("### 7.2. Свързани видеозаписи в локалната база\n");
        for v in video_contexts {
            out.push_str(&format!("- {}\n", v));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "# {}\n\n### 8.1. Справка в Wayback Machine и Internet Archive\n\
        - **Най-ранно архивно копие**: {}\n\
        - **Най-ново архивно копие**: {}\n\
        - **Състояние на архивната следа**: {}\n\n",
        headings[7],
        temporal_audit.oldest_capture.as_deref().unwrap_or("няма данни"),
        temporal_audit.newest_capture.as_deref().unwrap_or("няма данни"),
        temporal_audit.scrubbed_or_altered_risk
    ));

    if !temporal_audit.historical_snapshots.is_empty() {
        out.push_str("### 8.2. Архивирани уеб страници\n");
        for s in &temporal_audit.historical_snapshots {
            out.push_str(&format!("- [{}] [{}]({}) (Оригинален адрес: {}, HTTP {})\n", s.service, s.timestamp, s.url, s.original_url, s.status));
        }
        out.push('\n');
    }

    if !temporal_audit.internet_archive_docs.is_empty() {
        out.push_str("### 8.3. Документи и записи в Internet Archive\n");
        for doc in &temporal_audit.internet_archive_docs {
            out.push_str(&format!("- **[{}]({})** (Дата: {}, Вид: {})\n", doc.title, doc.detail_url, doc.date.as_deref().unwrap_or("без дата"), doc.mediatype.as_deref().unwrap_or("документ")));
        }
        out.push('\n');
    }

    if let Some(ref wb) = temporal_audit.wiki_bio {
        if !wb.revisions.is_empty() {
            out.push_str("### 8.4. Последни редакции в Уикипедия\n");
            for r in &wb.revisions {
                let comm = if r.comment.is_empty() { "без бележка".to_string() } else { r.comment.clone() };
                out.push_str(&format!("- **{}** — `{}`: *{}*\n", r.timestamp, r.user, comm));
            }
            out.push('\n');
        }
    }

    out.push_str(&format!(
        "# {}\n\n### 9.1. Аналитично обобщение\n\
        Настоящата справка за **{}** ({}) обединява проверени данни от публични регистри, официални институционални източници, медийни публикации и уеб архиви. Всички посочени факти следва да се разглеждат във връзка с цитираните първични документи.\n\n\
        ### 9.2. Списък на проверените източници\n",
        headings[8],
        name,
        entity_type_bg_label(entity_type)
    ));

    let mut item_no = 1usize;
    if let Some(ref wb) = temporal_audit.wiki_bio {
        out.push_str(&format!("{}. Уикипедия ({}) — https://{}.wikipedia.org/wiki/{}\n", item_no, wb.title, wb.language, urlencoding::encode(&wb.title)));
        item_no += 1;
    }
    for d in docs {
        if d.url.starts_with("http") {
            out.push_str(&format!("{}. {} — {}\n", item_no, d.title, d.url));
            item_no += 1;
        }
    }
    for snap in &temporal_audit.historical_snapshots {
        out.push_str(&format!("{}. Wayback Machine ({}) — {}\n", item_no, snap.original_url, snap.url));
        item_no += 1;
    }
    for doc in &temporal_audit.internet_archive_docs {
        out.push_str(&format!("{}. Internet Archive: {} — {}\n", item_no, doc.title, doc.detail_url));
        item_no += 1;
    }

    out
}

async fn handle_get_person_dossier(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<i64>,
) -> Response {
    if let Ok(Some(p)) = ctx.db.get_person_by_id(id).await {
        if let Some(json_str) = p.detailed_profile_json {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                if let Some(md) = val.get("report_markdown").and_then(|v| v.as_str()) {
                    return (
                        [(axum::http::header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
                        md.to_string(),
                    ).into_response();
                }
            }
        }
        if let Some(sum) = p.summary {
            return (
                [(axum::http::header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
                format!("# Аналитична справка: {}\n\n{}", p.canonical_name, sum),
            ).into_response();
        }
    }
    (StatusCode::NOT_FOUND, "Dossier not found").into_response()
}

async fn handle_get_person_intel_stream(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<i64>,
) -> Response {
    match ctx.db.get_intel_events_for_person(id, 60).await {
        Ok(events) => Json(serde_json::json!({ "events": events })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn handle_refresh_person_intel_stream(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<i64>,
) -> Response {
    let person = match ctx.db.get_person_by_id(id).await {
        Ok(Some(p)) => p,
        Ok(None) => return (StatusCode::NOT_FOUND, "Person not found").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let stream = leadfinder_providers::DynamicNewsStream::new();
    let events = stream.fetch_events_for_entity(id, &person.canonical_name, &person.variants, 12).await.unwrap_or_default();
    let mut added_count = 0usize;
    for ev in &events {
        if let Ok(_) = ctx.db.record_intel_event(id, &ev.source_name, &ev.title, &ev.url, &ev.snippet).await {
            added_count += 1;
        }
    }
    let _ = ctx.db.touch_person_monitored(id).await;

    let updated_events = ctx.db.get_intel_events_for_person(id, 60).await.unwrap_or_default();
    Json(serde_json::json!({
        "events": updated_events,
        "newly_added": added_count,
        "monitored_at": chrono::Utc::now().to_rfc3339()
    })).into_response()
}

async fn handle_get_person_sanctions(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<i64>,
) -> Response {
    let person = match ctx.db.get_person_by_id(id).await {
        Ok(Some(p)) => p,
        Ok(None) => return (StatusCode::NOT_FOUND, "Person not found").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    if let Ok(Some((matches, checked_at))) = ctx.db.get_sanctions_matches(id).await {
        return Json(serde_json::json!({
            "matches": matches,
            "checked_at": checked_at,
            "cached": true
        })).into_response();
    }

    let matcher = leadfinder_providers::SanctionsMatcher::new();
    let matches = matcher.check_entity(&person.canonical_name, &person.variants).await.unwrap_or_default();
    let _ = ctx.db.save_sanctions_matches(id, &matches).await;
    let now = chrono::Utc::now().to_rfc3339();

    Json(serde_json::json!({
        "matches": matches,
        "checked_at": now,
        "cached": false
    })).into_response()
}

async fn handle_get_person_wayback(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<i64>,
) -> Response {
    let person = match ctx.db.get_person_by_id(id).await {
        Ok(Some(p)) => p,
        Ok(None) => return (StatusCode::NOT_FOUND, "Person not found").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    if let Some(ref json_str) = person.detailed_profile_json {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
            if let Some(ta) = val.get("temporal_audit") {
                if !ta.is_null() {
                    return Json(serde_json::json!({
                        "ok": true,
                        "temporal_audit": ta,
                        "cached": true
                    })).into_response();
                }
            }
        }
    }

    let events = ctx.db.get_intel_events_for_person(id, 20).await.unwrap_or_default();
    let candidate_urls: Vec<String> = events.into_iter().map(|e| e.url).collect();
    let web_archive = leadfinder_providers::WebArchiveEngine::new();
    let audit = web_archive.run_aegis_temporal_audit(&person.canonical_name, &person.entity_type, &candidate_urls).await;
    Json(serde_json::json!({
        "ok": true,
        "temporal_audit": audit,
        "cached": false
    })).into_response()
}

async fn handle_refresh_person_wayback(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<i64>,
) -> Response {
    let person = match ctx.db.get_person_by_id(id).await {
        Ok(Some(p)) => p,
        Ok(None) => return (StatusCode::NOT_FOUND, "Person not found").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let events = ctx.db.get_intel_events_for_person(id, 20).await.unwrap_or_default();
    let candidate_urls: Vec<String> = events.into_iter().map(|e| e.url).collect();
    let web_archive = leadfinder_providers::WebArchiveEngine::new();
    let audit = web_archive.run_aegis_temporal_audit(&person.canonical_name, &person.entity_type, &candidate_urls).await;

    if let Some(ref json_str) = person.detailed_profile_json {
        if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(json_str) {
            if let Some(obj) = val.as_object_mut() {
                obj.insert("temporal_audit".to_string(), serde_json::to_value(&audit).unwrap_or(serde_json::Value::Null));
                if let Ok(new_json) = serde_json::to_string(&obj) {
                    let _ = ctx.db.update_person_research_data(
                        id,
                        person.role.as_deref(),
                        person.summary.as_deref(),
                        &person.tags,
                        Some(&new_json),
                        person.public_figure,
                        person.category.as_deref(),
                        Some(&person.entity_type),
                    ).await;
                }
            }
        }
    }

    Json(serde_json::json!({
        "ok": true,
        "temporal_audit": audit,
        "cached": false
    })).into_response()
}

async fn handle_list_people(
    State(ctx): State<Arc<PipelineContext>>,
    Query(q): Query<ListPeopleQuery>,
) -> Response {
    match ctx.db.list_all_people(
        q.query.as_deref(),
        q.tag.as_deref(),
        q.category.as_deref(),
        q.sort_by.as_deref(),
        q.entity_type.as_deref(),
    ).await {
        Ok(people) => Json(people).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ).into_response(),
    }
}

async fn handle_get_person(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<i64>,
) -> Response {
    match ctx.db.get_person_by_id(id).await {
        Ok(Some(p)) => {
            let (total, contradicted, supported) = ctx.db.get_person_claims_stats(id).await.unwrap_or((0, 0, 0));
            Json(serde_json::json!({
                "person": p,
                "claims_stats": {
                    "total": total,
                    "contradicted": contradicted,
                    "supported": supported,
                }
            })).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Person not found" })),
        ).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ).into_response(),
    }
}

async fn handle_create_person(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<CreatePersonRequest>,
) -> Response {
    let name = body.canonical_name.trim();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "canonical_name is required" })),
        ).into_response();
    }
    let tags = body.tags.unwrap_or_default();
    let rating = body.rating.unwrap_or(50);
    let pub_fig = body.public_figure.unwrap_or(true);
    let detected_type = leadfinder_providers::detect_entity_type(name, body.role.as_deref());
    let ent_type = body.entity_type.as_deref().unwrap_or(detected_type);
    match ctx.db.create_person(
        name,
        body.role.as_deref(),
        body.category.as_deref(),
        &tags,
        rating,
        body.notes.as_deref(),
        pub_fig,
        Some(ent_type),
    ).await {
        Ok(id) => Json(serde_json::json!({ "ok": true, "id": id })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ).into_response(),
    }
}

async fn handle_update_person(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<i64>,
    Json(body): Json<UpdatePersonRequest>,
) -> Response {
    let name = body.canonical_name.trim();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "canonical_name is required" })),
        ).into_response();
    }
    let tags = body.tags.unwrap_or_default();
    let rating = body.rating.unwrap_or(50);
    let pub_fig = body.public_figure.unwrap_or(true);
    match ctx.db.update_person_full(
        id,
        name,
        body.role.as_deref(),
        body.category.as_deref(),
        &tags,
        rating,
        body.notes.as_deref(),
        pub_fig,
        body.summary.as_deref(),
        body.detailed_profile_json.as_deref(),
        body.entity_type.as_deref(),
    ).await {
        Ok(()) => Json(serde_json::json!({ "ok": true, "id": id })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ).into_response(),
    }
}

async fn handle_delete_person(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<i64>,
) -> Response {
    match ctx.db.delete_person(id).await {
        Ok(()) => Json(serde_json::json!({ "ok": true, "deleted": id })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ).into_response(),
    }
}

async fn handle_auto_classify_people(
    State(ctx): State<Arc<PipelineContext>>,
) -> Response {
    let all = match ctx.db.list_all_people(None, None, None, None, None).await {
        Ok(list) => list,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            ).into_response();
        }
    };

    let mut updated = 0usize;
    let mut pruned = 0usize;
    let total = all.len();

    for p in all {
        if leadfinder_providers::is_extracted_noise(&p.canonical_name) {
            let _ = ctx.db.delete_person(p.id).await;
            pruned += 1;
            continue;
        }

        let detected = leadfinder_providers::detect_entity_type(&p.canonical_name, p.role.as_deref());
        let current_type = p.entity_type.as_str();

        let tag_to_add = match detected {
            "institution" => Some("институция"),
            "channel" => Some("канал"),
            "country" => Some("държава"),
            "group" => Some("политическа партия"),
            "event" => Some("събитие"),
            "infrastructure" => Some("инфраструктура"),
            "building" => Some("сграда"),
            _ => None,
        };

        let cat_to_set = match detected {
            "institution" => Some("Институция"),
            "channel" => Some("Медийна личност"),
            "country" => Some("Държава"),
            "group" => Some("Общественик"),
            "event" => Some("Събитие"),
            "infrastructure" => Some("Инфраструктурен обект"),
            "building" => Some("Сграда / Обект"),
            _ => None,
        };

        if detected != current_type || p.category.is_none() {
            let _ = ctx.db.update_person_classification(
                p.id,
                detected,
                cat_to_set,
                tag_to_add,
            ).await;
            updated += 1;
        }
    }

    Json(serde_json::json!({
        "ok": true,
        "total": total,
        "updated": updated,
        "pruned": pruned,
    })).into_response()
}

#[derive(Deserialize)]
struct ApplyDedupRequest {
    #[serde(alias = "action_type")]
    action: String,
    source_id: i64,
    target_id: Option<i64>,
    target_ids: Option<Vec<i64>>,
    target_names: Option<Vec<String>>,
    tag: Option<String>,
}

async fn handle_get_dedup_proposals(
    State(ctx): State<Arc<PipelineContext>>,
) -> Response {
    let all = match ctx.db.list_all_people(None, None, None, None, None).await {
        Ok(list) => list,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            ).into_response();
        }
    };

    let all_pairs: Vec<(i64, String)> = all.iter().map(|p| (p.id, p.canonical_name.clone())).collect();
    let mut proposals = Vec::new();

    for p in &all {
        if let Some(prop) = leadfinder_providers::generate_dedup_proposals_for_person(p.id, &p.canonical_name, &all_pairs) {
            proposals.push(prop);
        }
    }

    Json(serde_json::json!({
        "ok": true,
        "proposals": proposals,
        "total": proposals.len()
    })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct ClaimFeedbackRequest {
    pub video_id: String,
    pub rating: String,
    pub suggested_verdict: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DirectMergePeopleRequest {
    pub source_id: i64,
    pub target_id: i64,
    pub custom_name: Option<String>,
}

async fn handle_direct_merge_people(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<DirectMergePeopleRequest>,
) -> Response {
    if body.source_id == body.target_id {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "source_id and target_id must be different" })),
        ).into_response();
    }
    if let Err(e) = ctx.db.merge_people_with_name(body.source_id, body.target_id, body.custom_name.as_deref()).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ).into_response();
    }
    Json(serde_json::json!({ "ok": true })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct ClaimRedactRequest {
    pub video_id: String,
    pub text_bg: String,
    pub notes: Option<String>,
}

async fn handle_claim_redact(
    State(ctx): State<Arc<PipelineContext>>,
    Path(claim_id): Path<i64>,
    Json(body): Json<ClaimRedactRequest>,
) -> Response {
    let text = body.text_bg.trim().to_string();
    if text.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "text_bg must not be empty" })),
        ).into_response();
    }
    if let Err(e) = ctx.db.update_claim_text(claim_id, &text).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ).into_response();
    }
    if let Some(notes) = body.notes.as_deref().filter(|n| !n.trim().is_empty()) {
        let _ = ctx.db.save_claim_feedback(claim_id, &body.video_id, "edited", None, Some(notes)).await;
    }
    let run_ctx = Arc::clone(&ctx);
    let vid = body.video_id.clone();
    tokio::spawn(async move {
        let _ = run_ctx.verify_claim_with_retrieval(&vid, claim_id).await;
        if let Ok(Some(v)) = run_ctx.db.get_video(&vid).await {
            run_ctx.emit(CoreEvent::VideoUpdated {
                video_id: vid,
                state: v.state,
                lane: v.lane,
            });
        }
    });
    Json(serde_json::json!({ "ok": true, "claim_id": claim_id })).into_response()
}

async fn handle_claim_feedback(
    State(ctx): State<Arc<PipelineContext>>,
    Path(claim_id): Path<i64>,
    Json(body): Json<ClaimFeedbackRequest>,
) -> Response {
    if let Err(e) = ctx.db.save_claim_feedback(
        claim_id,
        &body.video_id,
        &body.rating,
        body.suggested_verdict.as_deref(),
        body.notes.as_deref(),
    ).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ).into_response();
    }
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn handle_apply_dedup_proposal(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<ApplyDedupRequest>,
) -> Response {
    let resolved_target_id = body
        .target_id
        .or_else(|| body.target_ids.as_ref().and_then(|v| v.first().copied()));

    match body.action.as_str() {
        "merge" | "Merge" => {
            if let Some(target_id) = resolved_target_id {
                let custom_name = body.target_names.as_ref().and_then(|names| names.first()).map(|s| s.as_str());
                if let Err(e) = ctx.db.merge_people_with_name(body.source_id, target_id, custom_name).await {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": e.to_string() })),
                    ).into_response();
                }
            } else if let Some(target_names) = &body.target_names {
                if let Some(first_target) = target_names.first() {
                    let tid = match ctx.db.split_compound_person(body.source_id, &[first_target.clone()]).await {
                        Ok(ids) => ids.into_iter().next().unwrap_or(0),
                        Err(e) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({ "error": e.to_string() })),
                            ).into_response();
                        }
                    };
                    return Json(serde_json::json!({ "ok": true, "target_id": tid })).into_response();
                }
            }
        }
        "split" | "SplitCompound" | "split_compound" => {
            let names = body.target_names.unwrap_or_default();
            if let Err(e) = ctx.db.split_compound_person(body.source_id, &names).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                ).into_response();
            }
        }
        "cleanup_tag" | "CleanupTag" => {
            let target_name = body.target_names.and_then(|names| names.into_iter().next()).unwrap_or_default();
            let tag = body.tag.unwrap_or_else(|| "политика".to_string());
            if let Err(e) = ctx.db.cleanup_person_as_tag(body.source_id, &target_name, &tag).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                ).into_response();
            }
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "unknown action" })),
            ).into_response();
        }
    }

    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn handle_auto_dedup_people(
    State(ctx): State<Arc<PipelineContext>>,
) -> Response {
    let all = match ctx.db.list_all_people(None, None, None, None, None).await {
        Ok(list) => list,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            ).into_response();
        }
    };

    let all_pairs: Vec<(i64, String)> = all.iter().map(|p| (p.id, p.canonical_name.clone())).collect();
    let mut proposals = Vec::new();

    for p in &all {
        if let Some(prop) = leadfinder_providers::generate_dedup_proposals_for_person(p.id, &p.canonical_name, &all_pairs) {
            proposals.push(prop);
        }
    }

    let mut merged = 0usize;
    let mut split = 0usize;
    let mut cleaned_tags = 0usize;
    let mut details = Vec::new();

    for prop in proposals {
        if prop.confidence < 0.90 {
            continue;
        }
        match prop.action_type {
            leadfinder_core::DedupActionType::Merge => {
                if let Some(tid) = prop.target_ids.first() {
                    if ctx.db.merge_people(prop.source_id, *tid).await.is_ok() {
                        merged += 1;
                        details.push(format!("Обединен: '{}' -> '{}'", prop.source_name, prop.target_names.first().cloned().unwrap_or_default()));
                    }
                }
            }
            leadfinder_core::DedupActionType::SplitCompound => {
                if ctx.db.split_compound_person(prop.source_id, &prop.target_names).await.is_ok() {
                    split += 1;
                    details.push(format!("Разделен: '{}' -> {}", prop.source_name, prop.target_names.join(" + ")));
                }
            }
            leadfinder_core::DedupActionType::CleanupTag => {
                if let (Some(target), Some(tag)) = (prop.target_names.first(), prop.tag.as_deref()) {
                    if ctx.db.cleanup_person_as_tag(prop.source_id, target, tag).await.is_ok() {
                        cleaned_tags += 1;
                        details.push(format!("Преобразуван: '{}' -> '{}' (#{tag})", prop.source_name, target));
                    }
                }
            }
        }
    }

    Json(serde_json::json!({
        "ok": true,
        "summary": {
            "merged": merged,
            "split": split,
            "tagged": cleaned_tags,
            "total_processed": merged + split + cleaned_tags,
            "merged_count": merged,
            "split_count": split,
            "cleaned_tags_count": cleaned_tags,
            "details": details
        }
    })).into_response()
}

async fn handle_get_notifications(
    State(ctx): State<Arc<PipelineContext>>,
) -> Response {
    let all = ctx.db.list_all_people(None, None, None, None, None).await.unwrap_or_default();
    let all_pairs: Vec<(i64, String)> = all.iter().map(|p| (p.id, p.canonical_name.clone())).collect();
    let mut dedup_questions_count = 0usize;

    for p in &all {
        if leadfinder_providers::generate_dedup_proposals_for_person(p.id, &p.canonical_name, &all_pairs).is_some() {
            dedup_questions_count += 1;
        }
    }

    let mut notifications = Vec::new();

    let mut active_list: Vec<crate::engine::ActiveResearchTask> = {
        let map = ctx.active_tasks.read().await;
        map.values().cloned().collect()
    };
    active_list.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    let ongoing_count = active_list.len();

    for task in active_list {
        notifications.push(serde_json::json!({
            "id": format!("ongoing_{}", task.id),
            "type": "ongoing",
            "title": task.title,
            "message": task.stage,
            "timestamp": task.started_at,
            "progress_pct": task.progress_pct,
            "subject_id": task.subject_id,
            "video_id": task.video_id,
            "subject_name": task.subject_name,
        }));
    }

    if let Ok(qval) = ctx.db.queue_counts().await {
        let queued = qval
            .get("queued")
            .or_else(|| qval.get("pending"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let running = qval.get("running").and_then(|v| v.as_i64()).unwrap_or(0);
        if running > 0 || queued > 0 {
            notifications.push(serde_json::json!({
                "id": "pipeline_queue_active",
                "type": "ongoing",
                "title": format!("Обработка на видео опашка ({} в ход, {} чакащи)", running, queued),
                "message": "Изпълнява се транскрибиране, извличане на твърдения и проверка по източници.",
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "progress_pct": if running > 0 { 60 } else { 20 },
            }));
        }
    }

    if dedup_questions_count > 0 {
        notifications.push(serde_json::json!({
            "id": "pending_dedup",
            "type": "question",
            "title": format!("Открити са {} дублирани или съставни записа", dedup_questions_count),
            "message": "Открити са повтарящи се профили или съставни имена. Прегледайте предложенията в раздела за дубликати.",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "count": dedup_questions_count
        }));
    }

    let mut researched_people: Vec<_> = all.iter().filter(|p| p.researched_at.is_some()).collect();
    researched_people.sort_by(|a, b| b.researched_at.cmp(&a.researched_at));
    for rp in researched_people.into_iter().take(8) {
        let short_summary = rp
            .summary
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.chars().take(160).collect::<String>())
            .unwrap_or_else(|| "Справката съдържа данни от публични регистри, медийни публикации, санкционни списъци и уеб архиви.".to_string());
        notifications.push(serde_json::json!({
            "id": format!("dossier_{}", rp.id),
            "type": "success",
            "title": format!("Изготвена аналитична справка: {}", rp.canonical_name),
            "message": short_summary,
            "timestamp": rp.researched_at.clone().unwrap_or_default(),
            "subject_id": rp.id,
            "subject_name": rp.canonical_name,
        }));
    }

    Json(serde_json::json!({
        "ok": true,
        "unread_count": ongoing_count + dedup_questions_count + notifications.len().min(3),
        "ongoing_count": ongoing_count,
        "questions_count": dedup_questions_count,
        "notifications": notifications
    })).into_response()
}

async fn handle_reassign_video_person(
    State(ctx): State<Arc<PipelineContext>>,
    Path((video_id, person_id)): Path<(String, i64)>,
    Json(body): Json<ReassignVideoPersonRequest>,
) -> Response {
    let target_person_id = if let Some(new_id) = body.new_person_id {
        new_id
    } else if let Some(name) = body.canonical_name.filter(|n| !n.trim().is_empty()) {
        let tags = body.tags.unwrap_or_default();
        let detected = leadfinder_providers::detect_entity_type(&name, body.role.as_deref());
        let ent = body.entity_type.as_deref().unwrap_or(detected);
        match ctx.db.create_person(
            &name,
            body.role.as_deref(),
            body.category.as_deref(),
            &tags,
            75,
            None,
            true,
            Some(ent),
        ).await {
            Ok(new_id) => new_id,
            Err(e) => return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            ).into_response(),
        }
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Either new_person_id or canonical_name must be provided" })),
        ).into_response();
    };

    match ctx.db.reassign_video_person(&video_id, person_id, target_person_id).await {
        Ok(()) => Json(serde_json::json!({ "ok": true, "video_id": video_id, "old_person_id": person_id, "new_person_id": target_person_id })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ).into_response(),
    }
}

async fn handle_remove_video_person(
    State(ctx): State<Arc<PipelineContext>>,
    Path((video_id, person_id)): Path<(String, i64)>,
) -> Response {
    match ctx.db.remove_person_from_video(&video_id, person_id).await {
        Ok(()) => Json(serde_json::json!({ "ok": true, "video_id": video_id, "removed_person_id": person_id })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ).into_response(),
    }
}

async fn handle_list_seeds(State(ctx): State<Arc<PipelineContext>>) -> Response {
    match ctx.db.list_seeds(false).await {
        Ok(seeds) => Json(seeds).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_upsert_seed(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<SeedUpsertRequest>,
) -> Response {
    match ctx
        .db
        .upsert_seed(&body.kind, &body.value, body.topic.as_deref(), body.active)
        .await
    {
        Ok(seed) => Json(seed).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_delete_seed(
    State(ctx): State<Arc<PipelineContext>>,
    Path(id): Path<i64>,
) -> Response {
    match ctx.db.delete_seed(id).await {
        Ok(()) => Json(serde_json::json!({ "deleted": id })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_list_creators(State(ctx): State<Arc<PipelineContext>>) -> Response {
    match ctx.db.list_creators().await {
        Ok(creators) => Json(creators).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_set_watchlist(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<WatchlistRequest>,
) -> Response {
    match ctx.db.set_creator_watchlist(&body.handle, body.watchlist).await {
        Ok(creator) => Json(creator).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize, Default)]
struct AssessRiskRequest {
    display_name: Option<String>,
    force_web: Option<bool>,
}

async fn handle_assess_creator_risk(
    State(ctx): State<Arc<PipelineContext>>,
    Path(handle): Path<String>,
    body: Option<Json<AssessRiskRequest>>,
) -> Response {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let force = req.force_web.unwrap_or(true);
    let task_id = format!("creator_risk:{}", handle);
    ctx.upsert_active_task(crate::engine::ActiveResearchTask {
        id: task_id.clone(),
        kind: "creator_risk".to_string(),
        subject_id: None,
        video_id: None,
        subject_name: format!("@{}", handle),
        title: format!("Проверка на профила: @{}", handle),
        stage: "Търсене на опровержения и публикации за автора...".to_string(),
        progress_pct: 50,
        started_at: chrono::Utc::now().to_rfc3339(),
    }).await;
    let _cleanup = ActiveTaskCleanup { ctx: Arc::clone(&ctx), task_id };
    match crate::creator_risk::assess_creator_risk(&ctx, &handle, req.display_name.as_deref(), force).await {
        Ok(assessment) => Json(assessment).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_get_profile(State(ctx): State<Arc<PipelineContext>>) -> Response {
    match ctx.db.get_profile().await {
        Ok(prof) => Json(prof).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_calibrate(
    State(ctx): State<Arc<PipelineContext>>,
    body: Option<Json<CalibrateRequest>>,
) -> Response {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let power_info = leadfinder_probe::detect_power_info();
    let power_source = req
        .power_source
        .unwrap_or(power_info.power_source)
        .to_lowercase();

    {
        let state = ctx.benchmark_state.read().await;
        if state.is_running && !req.force_rerun {
            return Json(serde_json::json!({
                "already_running": true,
                "progress": state.clone(),
            }))
            .into_response();
        }
    }

    {
        let mut state = ctx.benchmark_state.write().await;
        state.is_running = true;
        state.power_source = power_source.clone();
        state.current_stage = "detect_hw".to_string();
        state.stage_index = 1;
        state.total_stages = 4;
        state.progress_pct = 5;
        state.status_text = "Стартиране на калибриране...".to_string();
        state.status_text_en = "Starting calibration...".to_string();
        state.current_model = None;
        state.asr_results.clear();
        state.llm_results.clear();
        state.final_capacity = None;
        state.error = None;
        state.started_at = Some(chrono::Utc::now().to_rfc3339());
        state.completed_at = None;
    }

    let initial_payload = serde_json::to_value(&*ctx.benchmark_state.read().await).unwrap_or_default();
    let _ = ctx.events_tx.send(leadfinder_core::CoreEvent::BenchmarkProgress(initial_payload));

    let ctx_clone = ctx.clone();
    let power_source_clone = power_source.clone();
    tokio::spawn(async move {
        let cfg = ctx_clone.config.read().await.clone();
        let bstate = ctx_clone.benchmark_state.clone();
        let ev_tx = ctx_clone.events_tx.clone();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<leadfinder_probe::BenchmarkProgressUpdate>();
        let bstate_worker = bstate.clone();
        let ev_tx_worker = ev_tx.clone();
        let updater = tokio::spawn(async move {
            while let Some(update) = rx.recv().await {
                let mut st = bstate_worker.write().await;
                st.current_stage = update.stage.clone();
                st.stage_index = update.stage_index;
                st.total_stages = update.total_stages;
                st.progress_pct = update.percent;
                st.status_text = update.message_bg.clone();
                st.status_text_en = update.message_en.clone();
                if let Some(ref m) = update.current_model {
                    st.current_model = Some(m.clone());
                }
                if let Some(ref asr) = update.asr_benchmark {
                    if !st.asr_results.iter().any(|b| b.model == asr.model) {
                        st.asr_results.push(asr.clone());
                    }
                }
                if let Some(ref llm) = update.llm_benchmark {
                    if !st.llm_results.iter().any(|b| b.model == llm.model) {
                        st.llm_results.push(llm.clone());
                    }
                }
                let payload = serde_json::to_value(&*st).unwrap_or_default();
                let _ = ev_tx_worker.send(leadfinder_core::CoreEvent::BenchmarkProgress(payload));
            }
        });

        let progress_cb: leadfinder_probe::BenchmarkProgressCallback = {
            let tx = tx.clone();
            Arc::new(move |update: leadfinder_probe::BenchmarkProgressUpdate| {
                let _ = tx.send(update);
            })
        };

        let cal_res = leadfinder_probe::run_calibration_for_power(
            &ctx_clone.paths,
            &cfg,
            &ctx_clone.db,
            req.force_cpu,
            Some(&power_source_clone),
            Some(progress_cb),
        )
        .await;

        drop(tx);
        let _ = updater.await;

        match cal_res {
            Ok(profile) => {
                let mut st = bstate.write().await;
                st.is_running = false;
                st.current_stage = "done".to_string();
                st.progress_pct = 100;
                st.final_capacity = profile.capacity_per_day;
                st.completed_at = Some(chrono::Utc::now().to_rfc3339());
                st.status_text = format!(
                    "Готово! Измерен капацитет: {:.0} видеа/ден ({})",
                    profile.capacity_per_day.unwrap_or(0.0),
                    power_source_clone.to_uppercase()
                );
                st.status_text_en = format!(
                    "Complete! Measured capacity: {:.0} videos/day ({})",
                    profile.capacity_per_day.unwrap_or(0.0),
                    power_source_clone.to_uppercase()
                );
                let payload = serde_json::to_value(&*st).unwrap_or_default();
                let _ = ev_tx.send(leadfinder_core::CoreEvent::BenchmarkProgress(payload));
                let _ = ev_tx.send(leadfinder_core::CoreEvent::CalibrationUpdated {
                    capacity_per_day: profile.capacity_per_day,
                });
            }
            Err(e) => {
                let mut st = bstate.write().await;
                st.is_running = false;
                st.current_stage = "failed".to_string();
                st.error = Some(e.to_string());
                st.status_text = format!("Грешка при калибриране: {e}");
                st.status_text_en = format!("Calibration failed: {e}");
                let payload = serde_json::to_value(&*st).unwrap_or_default();
                let _ = ev_tx.send(leadfinder_core::CoreEvent::BenchmarkProgress(payload));
            }
        }
    });

    let current = ctx.benchmark_state.read().await.clone();
    Json(serde_json::json!({
        "started": true,
        "progress": current,
    }))
    .into_response()
}

async fn handle_calibrate_status(State(ctx): State<Arc<PipelineContext>>) -> Response {
    let progress = ctx.benchmark_state.read().await.clone();
    let power = leadfinder_probe::detect_power_info();
    let (ac, batt) = ctx.db.get_power_profiles().await.unwrap_or((None, None));
    let active = ctx.db.get_profile().await.ok().flatten();
    Json(CalibrateStatusResponse {
        is_running: progress.is_running,
        progress,
        power,
        ac_profile: ac,
        battery_profile: batt,
        active_profile: active,
    })
    .into_response()
}

async fn handle_calibrate_switch_power(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<SwitchPowerRequest>,
) -> Response {
    let prof = ctx.db.get_profile_for_power(&body.power_source).await.ok().flatten();
    if let Some(ref p) = prof {
        let _ = ctx.db.save_profile(p).await;
        let _ = ctx.events_tx.send(leadfinder_core::CoreEvent::CalibrationUpdated {
            capacity_per_day: p.capacity_per_day,
        });
    }
    Json(serde_json::json!({
        "power_source": body.power_source,
        "profile": prof,
    }))
    .into_response()
}

async fn handle_calibrate_overrides(
    State(ctx): State<Arc<PipelineContext>>,
    Json(overrides): Json<CalibrationOverrides>,
) -> Response {
    let cfg = ctx.config.read().await.clone();
    match apply_calibration_overrides(&ctx.db, &cfg, overrides).await {
        Ok(prof) => Json(prof).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_list_settings(State(ctx): State<Arc<PipelineContext>>) -> Response {
    match ctx.db.list_settings().await {
        Ok(map) => Json(map).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_update_settings(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<serde_json::Map<String, serde_json::Value>>,
) -> Response {
    for (k, v) in &body {
        if let Err(e) = ctx.db.set_setting(k, v).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
        if k == "cloud_enabled" {
            if let Some(flag) = v.as_bool() {
                ctx.config.write().await.providers.cloud_enabled = flag;
            }
        }
    }
    Json(serde_json::json!({ "updated": body.len() })).into_response()
}

#[derive(Debug, Serialize)]
pub struct GeminiStatusResponse {
    pub configured: bool,
    pub masked_key: Option<String>,
    pub cloud_enabled: bool,
    pub model: String,
    pub total_keys: usize,
    pub active_key_index: usize,
    pub keys: Vec<GeminiKeyInfo>,
}

#[derive(Debug, Deserialize)]
pub struct SetGeminiKeyRequest {
    pub key: Option<String>,
    pub keys: Option<Vec<String>>,
}

fn mask_api_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.len() <= 8 {
        return "********".to_string();
    }
    format!("{}...{}", &trimmed[..6], &trimmed[trimmed.len().saturating_sub(4)..])
}

async fn handle_gemini_status(
    State(ctx): State<Arc<PipelineContext>>,
) -> Response {
    let cfg = ctx.config.read().await;
    let resolved_key = leadfinder_core::resolve_secret("GEMINI_API_KEY");
    let pool_val = ctx.db.get_setting("gemini_keys_pool").await.ok().flatten();

    let mut keys_list: Vec<String> = Vec::new();
    let mut active_idx = 0usize;

    if let Some(val) = pool_val {
        if let Some(arr) = val.get("keys").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    let t = s.trim().to_string();
                    if !t.is_empty() && !keys_list.contains(&t) {
                        keys_list.push(t);
                    }
                }
            }
        }
        active_idx = val.get("active_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    }

    if keys_list.is_empty() {
        if let Some(ref k) = resolved_key {
            keys_list.push(k.clone());
        }
    }

    if active_idx >= keys_list.len() {
        active_idx = 0;
    }

    let configured = !keys_list.is_empty();
    let active_key = keys_list.get(active_idx).cloned().or(resolved_key);
    let masked_key = active_key.as_deref().map(mask_api_key);
    let cloud_enabled = cfg.providers.cloud_enabled;
    let model = cfg.providers.gemini_model.clone();

    let key_infos: Vec<GeminiKeyInfo> = keys_list
        .iter()
        .enumerate()
        .map(|(i, k)| GeminiKeyInfo {
            index: i,
            masked: mask_api_key(k),
            is_active: i == active_idx,
        })
        .collect();

    Json(GeminiStatusResponse {
        configured,
        masked_key,
        cloud_enabled,
        model,
        total_keys: keys_list.len(),
        active_key_index: active_idx,
        keys: key_infos,
    })
    .into_response()
}

async fn handle_set_gemini_key(
    State(ctx): State<Arc<PipelineContext>>,
    Json(body): Json<SetGeminiKeyRequest>,
) -> Response {
    let mut raw_keys = Vec::new();
    if let Some(ref k) = body.key {
        for part in k.split(|c| c == '\n' || c == ',' || c == ';' || c == '\r') {
            let t = part.trim().to_string();
            if !t.is_empty() && !raw_keys.contains(&t) {
                raw_keys.push(t);
            }
        }
    }
    if let Some(ref list) = body.keys {
        for k in list {
            let t = k.trim().to_string();
            if !t.is_empty() && !raw_keys.contains(&t) {
                raw_keys.push(t);
            }
        }
    }

    if raw_keys.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "API key cannot be empty" })),
        )
            .into_response();
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let mut valid_keys = Vec::new();
    let mut errors = Vec::new();

    for key in raw_keys {
        let test_url = format!("https://generativelanguage.googleapis.com/v1beta/models?key={}", key);
        match client.get(&test_url).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    valid_keys.push(key);
                } else {
                    let st = resp.status();
                    let err = resp.text().await.unwrap_or_default();
                    errors.push(format!("{}: HTTP {st} - {err}", mask_api_key(&key)));
                }
            }
            Err(e) => {
                errors.push(format!("{}: {e}", mask_api_key(&key)));
            }
        }
    }

    if valid_keys.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Validation failed for all keys: {}", errors.join("; "))
            })),
        )
            .into_response();
    }

    let active_key = valid_keys[0].clone();
    std::env::set_var("GEMINI_API_KEY", &active_key);
    let _ = leadfinder_core::store_secret_in_os_credential_store("GEMINI_API_KEY", &active_key);

    let pool = serde_json::json!({
        "keys": valid_keys,
        "active_index": 0,
    });
    let _ = ctx.db.set_setting("gemini_keys_pool", &pool).await;
    let _ = ctx.db.set_setting("cloud_enabled", &serde_json::json!(true)).await;
    let _ = ctx.db.set_setting("gemini_configured", &serde_json::json!(true)).await;
    ctx.config.write().await.providers.cloud_enabled = true;

    let key_infos: Vec<GeminiKeyInfo> = valid_keys
        .iter()
        .enumerate()
        .map(|(i, k)| GeminiKeyInfo {
            index: i,
            masked: mask_api_key(k),
            is_active: i == 0,
        })
        .collect();

    Json(serde_json::json!({
        "ok": true,
        "total_keys": valid_keys.len(),
        "masked_key": mask_api_key(&active_key),
        "keys": key_infos,
        "cloud_enabled": true,
        "validation_warnings": if errors.is_empty() { None } else { Some(errors) }
    }))
    .into_response()
}

async fn handle_rotate_gemini_key(State(ctx): State<Arc<PipelineContext>>) -> Response {
    let pool_val = ctx.db.get_setting("gemini_keys_pool").await.ok().flatten();
    let Some(mut pool) = pool_val else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "No alternate keys available in pool" })),
        )
            .into_response();
    };

    let keys = match pool.get("keys").and_then(|v| v.as_array()) {
        Some(k) if k.len() > 1 => k.clone(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Pool has only 1 key or empty" })),
            )
                .into_response();
        }
    };

    let cur_idx = pool.get("active_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let next_idx = (cur_idx + 1) % keys.len();
    let next_key = keys.get(next_idx).and_then(|v| v.as_str()).unwrap_or("").to_string();

    pool["active_index"] = serde_json::json!(next_idx);
    let _ = ctx.db.set_setting("gemini_keys_pool", &pool).await;
    std::env::set_var("GEMINI_API_KEY", &next_key);
    let _ = leadfinder_core::store_secret_in_os_credential_store("GEMINI_API_KEY", &next_key);

    Json(serde_json::json!({
        "ok": true,
        "rotated": true,
        "active_index": next_idx,
        "masked_key": mask_api_key(&next_key),
    }))
    .into_response()
}

async fn handle_delete_gemini_key(
    State(ctx): State<Arc<PipelineContext>>,
) -> Response {
    std::env::remove_var("GEMINI_API_KEY");
    let _ = leadfinder_core::store_secret_in_os_credential_store("GEMINI_API_KEY", "");
    let _ = ctx.db.set_setting("gemini_keys_pool", &serde_json::json!({ "keys": [], "active_index": 0 })).await;
    let _ = ctx.db.set_setting("cloud_enabled", &serde_json::json!(false)).await;
    let _ = ctx.db.set_setting("gemini_configured", &serde_json::json!(false)).await;
    ctx.config.write().await.providers.cloud_enabled = false;

    Json(serde_json::json!({
        "ok": true,
        "configured": false,
        "cloud_enabled": false
    }))
    .into_response()
}

async fn handle_system_shutdown(State(_ctx): State<Arc<PipelineContext>>) -> Response {
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        std::process::exit(0);
    });
    Json(serde_json::json!({
        "ok": true,
        "terminating": true
    }))
    .into_response()
}

async fn handle_system_shutdown_when_idle(State(ctx): State<Arc<PipelineContext>>) -> Response {
    ctx.auto_terminate_when_idle
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let counts = ctx
        .db
        .queue_counts()
        .await
        .unwrap_or_else(|_| serde_json::json!({}));
    Json(serde_json::json!({
        "ok": true,
        "auto_terminate_enabled": true,
        "queue_counts": counts
    }))
    .into_response()
}

async fn handle_system_exit_status(State(ctx): State<Arc<PipelineContext>>) -> Response {
    let auto_term = ctx
        .auto_terminate_when_idle
        .load(std::sync::atomic::Ordering::Relaxed);
    let counts = ctx
        .db
        .queue_counts()
        .await
        .unwrap_or_else(|_| serde_json::json!({}));
    Json(serde_json::json!({
        "auto_terminate_when_idle": auto_term,
        "queue_counts": counts
    }))
    .into_response()
}

async fn handle_scale_status(State(ctx): State<Arc<PipelineContext>>) -> impl IntoResponse {
    let aimd = ctx.concurrency.snapshot().await;
    let mut sys = sysinfo::System::new();
    let pid = sysinfo::get_current_pid().ok();
    let rss = if let Some(p) = pid {
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
        sys.process(p).map(|proc| proc.memory()).unwrap_or(0)
    } else {
        0
    };

    let disks = sysinfo::Disks::new_with_refreshed_list();
    let free_disk = disks.iter().map(|d| d.available_space()).max().unwrap_or(0);
    let media_bytes = compute_media_bytes(&ctx.media_root());
    let breakers = ctx.guard.breaker_statuses().await;
    let open_breakers = breakers.iter().filter(|b| b.is_open).count();

    Json(serde_json::json!({
        "aimd": aimd,
        "rss_bytes": rss,
        "disk_free_bytes": free_disk,
        "media_bytes": media_bytes,
        "open_breakers": open_breakers,
    }))
}

fn compute_media_bytes(media_root: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(media_root) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                total += compute_media_bytes(&p);
            } else if let Ok(meta) = p.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

async fn handle_metrics(State(ctx): State<Arc<PipelineContext>>) -> impl IntoResponse {
    let queue = ctx.db.queue_counts().await.unwrap_or_else(|_| serde_json::json!({}));
    let breakers = ctx.guard.breaker_statuses().await;
    let open_breakers = breakers.iter().filter(|b| b.is_open).count();

    let queued = queue.get("queued").and_then(|v| v.as_i64()).unwrap_or(0);
    let running = queue.get("running").and_then(|v| v.as_i64()).unwrap_or(0);
    let succeeded = queue.get("succeeded").and_then(|v| v.as_i64()).unwrap_or(0);
    let failed = queue.get("failed").and_then(|v| v.as_i64()).unwrap_or(0);

    let aimd = ctx.concurrency.snapshot().await;

    let mut sys = sysinfo::System::new();
    let pid = sysinfo::get_current_pid().ok();
    let rss = if let Some(p) = pid {
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
        sys.process(p).map(|proc| proc.memory()).unwrap_or(0)
    } else {
        0
    };

    let disks = sysinfo::Disks::new_with_refreshed_list();
    let free_disk = disks.iter().map(|d| d.available_space()).max().unwrap_or(0);
    let media_bytes = compute_media_bytes(&ctx.media_root());

    let mut text = format!(
        "leadfinder_jobs_queued {queued}\nleadfinder_jobs_running {running}\nleadfinder_jobs_succeeded {succeeded}\nleadfinder_jobs_failed {failed}\nleadfinder_circuit_breakers_open {open_breakers}\nleadfinder_process_memory_rss_bytes {rss}\nleadfinder_disk_free_bytes {free_disk}\nleadfinder_media_bytes {media_bytes}\nleadfinder_backpressure_active {}\n",
        if aimd.backpressure_active { 1 } else { 0 }
    );

    for (st, snap) in &aimd.stages {
        text.push_str(&format!(
            "leadfinder_aimd_concurrency{{stage=\"{}\"}} {}\n",
            st, snap.current_concurrency
        ));
        text.push_str(&format!(
            "leadfinder_aimd_active_tasks{{stage=\"{}\"}} {}\n",
            st, snap.active_tasks
        ));
        text.push_str(&format!(
            "leadfinder_aimd_latency_ms{{stage=\"{}\"}} {}\n",
            st, snap.last_latency_ms
        ));
    }

    if let Ok(usages) = ctx.db.get_today_provider_usage().await {
        for u in usages {
            text.push_str(&format!(
                "leadfinder_provider_requests_today{{provider=\"{}\",model=\"{}\"}} {}\n",
                u.provider, u.model, u.requests
            ));
            text.push_str(&format!(
                "leadfinder_provider_tokens_in_today{{provider=\"{}\",model=\"{}\"}} {}\n",
                u.provider, u.model, u.tokens_in
            ));
            text.push_str(&format!(
                "leadfinder_provider_tokens_out_today{{provider=\"{}\",model=\"{}\"}} {}\n",
                u.provider, u.model, u.tokens_out
            ));
            text.push_str(&format!(
                "leadfinder_provider_errors_429_today{{provider=\"{}\",model=\"{}\"}} {}\n",
                u.provider, u.model, u.errors_429
            ));
        }
    }

    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], text)
}

async fn handle_export(
    State(ctx): State<Arc<PipelineContext>>,
    Query(q): Query<ExportQuery>,
) -> Response {
    let fmt_str = q.format.as_deref().unwrap_or("markdown");
    let fmt = match crate::export::ExportFormat::parse_str(fmt_str) {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };
    match ctx.export_leads(q.lane.as_deref(), fmt).await {
        Ok(out) => match fmt {
            crate::export::ExportFormat::Markdown => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
                out,
            )
                .into_response(),
            crate::export::ExportFormat::Csv => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
                out,
            )
                .into_response(),
            crate::export::ExportFormat::Json => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                out,
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}


#[derive(Debug, Deserialize)]
pub struct DownloadModelRequest {
    pub model_id: String,
    pub url: String,
    pub filename: String,
    pub kind: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct ScanExternalAiServersRequest {
    pub custom_url: Option<String>,
}

fn format_file_size(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.2} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

async fn handle_list_models(State(ctx): State<Arc<PipelineContext>>) -> Response {
    let mut models: Vec<leadfinder_core::InstalledModelInfo> = Vec::new();

    let llm_dirs = [
        ctx.paths.models_dir.join("llm"),
        ctx.paths.static_root.join("models").join("llm"),
    ];
    for dir in &llm_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("gguf") {
                    let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
                    if !models.iter().any(|m| m.filename == fname) {
                        let meta = entry.metadata().ok();
                        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                        let modified = meta
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| {
                                chrono::DateTime::<chrono::Utc>::from(t)
                                    .to_rfc3339()
                                    .into()
                            })
                            .unwrap_or_default();
                        models.push(leadfinder_core::InstalledModelInfo {
                            id: format!("local_llm_{fname}"),
                            name: fname.clone(),
                            kind: "llm_gguf".to_string(),
                            filename: fname,
                            file_path: p.to_string_lossy().to_string(),
                            size_bytes: size,
                            size_display: format_file_size(size),
                            modified_at: modified,
                            recommended_roles: vec!["claim_extraction".to_string(), "fact_verification".to_string()],
                        });
                    }
                }
            }
        }
    }

    let whisper_dirs = [
        ctx.paths.models_dir.join("whisper"),
        ctx.paths.static_root.join("models").join("whisper"),
    ];
    for dir in &whisper_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("bin") {
                    let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
                    if !models.iter().any(|m| m.filename == fname) {
                        let meta = entry.metadata().ok();
                        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                        let modified = meta
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| {
                                chrono::DateTime::<chrono::Utc>::from(t)
                                    .to_rfc3339()
                                    .into()
                            })
                            .unwrap_or_default();
                        models.push(leadfinder_core::InstalledModelInfo {
                            id: format!("whisper_{fname}"),
                            name: fname.clone(),
                            kind: "whisper_bin".to_string(),
                            filename: fname,
                            file_path: p.to_string_lossy().to_string(),
                            size_bytes: size,
                            size_display: format_file_size(size),
                            modified_at: modified,
                            recommended_roles: vec!["asr".to_string()],
                        });
                    }
                }
            }
        }
    }

    models.push(leadfinder_core::InstalledModelInfo {
        id: "cloud_gemini_flash".to_string(),
        name: "gemini-2.5-flash".to_string(),
        kind: "cloud_gemini".to_string(),
        filename: "gemini-2.5-flash".to_string(),
        file_path: "Google AI Cloud".to_string(),
        size_bytes: 0,
        size_display: "Cloud API".to_string(),
        modified_at: "Online".to_string(),
        recommended_roles: vec!["claim_extraction".to_string(), "fact_verification".to_string(), "osint_dossier".to_string()],
    });

    models.push(leadfinder_core::InstalledModelInfo {
        id: "cloud_gemini_pro".to_string(),
        name: "gemini-1.5-pro".to_string(),
        kind: "cloud_gemini".to_string(),
        filename: "gemini-1.5-pro".to_string(),
        file_path: "Google AI Cloud".to_string(),
        size_bytes: 0,
        size_display: "Cloud API".to_string(),
        modified_at: "Online".to_string(),
        recommended_roles: vec!["fact_verification".to_string(), "osint_dossier".to_string()],
    });

    models.push(leadfinder_core::InstalledModelInfo {
        id: "offline_heuristics".to_string(),
        name: "heuristics".to_string(),
        kind: "offline_engine".to_string(),
        filename: "heuristics".to_string(),
        file_path: "Embedded Rust Rules".to_string(),
        size_bytes: 0,
        size_display: "0 MB (Built-in)".to_string(),
        modified_at: "Ready".to_string(),
        recommended_roles: vec!["claim_extraction".to_string()],
    });

    Json(serde_json::json!({
        "models": models,
        "models_dir": ctx.paths.models_dir.to_string_lossy(),
    }))
    .into_response()
}

async fn handle_get_model_catalog(State(ctx): State<Arc<PipelineContext>>) -> Response {
    let mut catalog = vec![
        leadfinder_core::CuratedModelCatalogItem {
            id: "bggpt-gemma-3-4b-it".to_string(),
            name: "BgGPT-Gemma-3-4B-IT-Q4_K_M".to_string(),
            author: "INSAIT Institute".to_string(),
            kind: "llm_gguf".to_string(),
            download_url: "https://huggingface.co/INSAIT-Institute/BgGPT-Gemma-3-4B-IT-GGUF/resolve/main/BgGPT-Gemma-3-4B-IT-Q4_K_M.gguf".to_string(),
            filename: "BgGPT-Gemma-3-4B-IT-Q4_K_M.gguf".to_string(),
            size_bytes: 2_489_893_728,
            size_display: "2.49 GB".to_string(),
            vram_estimate: "~3.5 GB VRAM".to_string(),
            description_bg: "Най-добрият специализиран български AI езиков модел. Перфектен за екстракция на твърдения и сложни формулировки.".to_string(),
            description_en: "State-of-the-art Bulgarian AI language model. Ideal for claim extraction and nuanced Bulgarian syntax.".to_string(),
            recommended_for: vec!["claim_extraction".to_string(), "fact_verification".to_string()],
            is_installed: false,
        },
        leadfinder_core::CuratedModelCatalogItem {
            id: "bggpt-gemma-2-2b-it".to_string(),
            name: "BgGPT-Gemma-2-2B-IT-v1.0.Q4_K_M".to_string(),
            author: "INSAIT Institute".to_string(),
            kind: "llm_gguf".to_string(),
            download_url: "https://huggingface.co/INSAIT-Institute/BgGPT-Gemma-2-2.6B-IT-v1.0-GGUF/resolve/main/BgGPT-Gemma-2-2B-IT-v1.0.Q4_K_M.gguf".to_string(),
            filename: "BgGPT-Gemma-2-2B-IT-v1.0.Q4_K_M.gguf".to_string(),
            size_bytes: 1_708_582_400,
            size_display: "1.71 GB".to_string(),
            vram_estimate: "~2.2 GB VRAM".to_string(),
            description_bg: "Ултра-лек и бърз български модел. Отличен за слаби лаптопи и режим на батерия.".to_string(),
            description_en: "Ultra-compact and fast Bulgarian model. Optimal for low-end laptops and battery mode.".to_string(),
            recommended_for: vec!["claim_extraction".to_string()],
            is_installed: false,
        },
        leadfinder_core::CuratedModelCatalogItem {
            id: "qwen-2.5-3b-instruct".to_string(),
            name: "Qwen2.5-3B-Instruct-Q4_K_M".to_string(),
            author: "Qwen Team (Alibaba)".to_string(),
            kind: "llm_gguf".to_string(),
            download_url: "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf".to_string(),
            filename: "qwen2.5-3b-instruct-q4_k_m.gguf".to_string(),
            size_bytes: 2_050_000_000,
            size_display: "2.05 GB".to_string(),
            vram_estimate: "~2.8 GB VRAM".to_string(),
            description_bg: "Глобален шампион по скорост и точност в 3B класа. Превъзходно структуриране в JSON.".to_string(),
            description_en: "Top-tier 3B model for speed and accuracy. Excellent JSON adherence.".to_string(),
            recommended_for: vec!["claim_extraction".to_string()],
            is_installed: false,
        },
        leadfinder_core::CuratedModelCatalogItem {
            id: "qwen-2.5-7b-instruct".to_string(),
            name: "Qwen2.5-7B-Instruct-Q4_K_M".to_string(),
            author: "Qwen Team (Alibaba)".to_string(),
            kind: "llm_gguf".to_string(),
            download_url: "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/qwen2.5-7b-instruct-q4_k_m.gguf".to_string(),
            filename: "qwen2.5-7b-instruct-q4_k_m.gguf".to_string(),
            size_bytes: 4_480_000_000,
            size_display: "4.48 GB".to_string(),
            vram_estimate: "~5.8 GB VRAM".to_string(),
            description_bg: "Мощен аналитичен модел за дълбоко логическо разсъждение и проверка на сложни твърдения.".to_string(),
            description_en: "Powerful analytical model for deep logical reasoning and fact verification.".to_string(),
            recommended_for: vec!["fact_verification".to_string(), "osint_dossier".to_string()],
            is_installed: false,
        },
        leadfinder_core::CuratedModelCatalogItem {
            id: "llama-3.2-3b-instruct".to_string(),
            name: "Llama-3.2-3B-Instruct-Q4_K_M".to_string(),
            author: "Meta AI".to_string(),
            kind: "llm_gguf".to_string(),
            download_url: "https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q4_K_M.gguf".to_string(),
            filename: "Llama-3.2-3B-Instruct-Q4_K_M.gguf".to_string(),
            size_bytes: 2_020_000_000,
            size_display: "2.02 GB".to_string(),
            vram_estimate: "~2.7 GB VRAM".to_string(),
            description_bg: "Бърз и надежден многоезичен модел от Meta за филтриране на факти.".to_string(),
            description_en: "Fast and reliable multilingual model from Meta for fact screening.".to_string(),
            recommended_for: vec!["claim_extraction".to_string()],
            is_installed: false,
        },
        leadfinder_core::CuratedModelCatalogItem {
            id: "whisper-large-v3-turbo".to_string(),
            name: "Whisper Large-v3-Turbo".to_string(),
            author: "OpenAI".to_string(),
            kind: "whisper_bin".to_string(),
            download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin".to_string(),
            filename: "ggml-large-v3-turbo-q5_0.bin".to_string(),
            size_bytes: 574_041_195,
            size_display: "574 MB".to_string(),
            vram_estimate: "~1.2 GB VRAM".to_string(),
            description_bg: "Препоръчителният флагман за разпознаване на българска реч с под 5% грешка (WER).".to_string(),
            description_en: "Recommended flagship Bulgarian speech recognition with <5% WER.".to_string(),
            recommended_for: vec!["asr".to_string()],
            is_installed: false,
        },
        leadfinder_core::CuratedModelCatalogItem {
            id: "whisper-small".to_string(),
            name: "Whisper Small".to_string(),
            author: "OpenAI".to_string(),
            kind: "whisper_bin".to_string(),
            download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin".to_string(),
            filename: "ggml-small.bin".to_string(),
            size_bytes: 487_601_967,
            size_display: "488 MB".to_string(),
            vram_estimate: "~1.0 GB VRAM".to_string(),
            description_bg: "Балансиран модел за по-слаби компютри и лаптопи на батерия.".to_string(),
            description_en: "Balanced ASR model for lower-spec machines and battery power.".to_string(),
            recommended_for: vec!["asr".to_string()],
            is_installed: false,
        },
        leadfinder_core::CuratedModelCatalogItem {
            id: "whisper-base".to_string(),
            name: "Whisper Base".to_string(),
            author: "OpenAI".to_string(),
            kind: "whisper_bin".to_string(),
            download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin".to_string(),
            filename: "ggml-base.bin".to_string(),
            size_bytes: 147_951_465,
            size_display: "148 MB".to_string(),
            vram_estimate: "~500 MB VRAM".to_string(),
            description_bg: "Ултра-лек ASR модел за максимална скорост на стари машини.".to_string(),
            description_en: "Ultra-lightweight ASR model for maximum speed on legacy systems.".to_string(),
            recommended_for: vec!["asr".to_string()],
            is_installed: false,
        },
    ];

    for item in &mut catalog {
        let sub = if item.kind == "whisper_bin" { "whisper" } else { "llm" };
        let check_paths = [
            ctx.paths.models_dir.join(sub).join(&item.filename),
            ctx.paths.static_root.join("models").join(sub).join(&item.filename),
        ];
        item.is_installed = check_paths.iter().any(|p| p.is_file());
    }

    Json(serde_json::json!({ "catalog": catalog })).into_response()
}

async fn handle_download_model(
    State(ctx): State<Arc<PipelineContext>>,
    Json(req): Json<DownloadModelRequest>,
) -> Response {
    if ctx.model_download_state.read().await.active {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "Another model download is already running." })),
        )
            .into_response();
    }

    let sub = if req.kind == "whisper" || req.filename.ends_with(".bin") { "whisper" } else { "llm" };
    let dest_dir = ctx.paths.models_dir.join(sub);
    let _ = std::fs::create_dir_all(&dest_dir);
    let final_dest = dest_dir.join(&req.filename);
    let part_dest = dest_dir.join(format!("{}.part", req.filename));

    ctx.model_download_cancel.store(false, Ordering::SeqCst);
    {
        let mut st = ctx.model_download_state.write().await;
        st.active = true;
        st.model_id = req.model_id.clone();
        st.filename = req.filename.clone();
        st.bytes_downloaded = 0;
        st.total_bytes = 0;
        st.progress_pct = 0;
        st.speed_bps = 0;
        st.status = "downloading".to_string();
        st.error_message = None;
    }

    let state_arc = ctx.model_download_state.clone();
    let cancel_arc = ctx.model_download_cancel.clone();
    let events_tx = ctx.events_tx.clone();

    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3600))
            .build()
            .unwrap_or_default();

        let resp_res = client.get(&req.url).send().await;
        let resp = match resp_res {
            Ok(r) => match r.error_for_status() {
                Ok(ok_r) => ok_r,
                Err(e) => {
                    let mut st = state_arc.write().await;
                    st.active = false;
                    st.status = "error".to_string();
                    st.error_message = Some(e.to_string());
                    return;
                }
            },
            Err(e) => {
                let mut st = state_arc.write().await;
                st.active = false;
                st.status = "error".to_string();
                st.error_message = Some(e.to_string());
                return;
            }
        };

        let total_size = resp.content_length().unwrap_or(0);
        {
            let mut st = state_arc.write().await;
            st.total_bytes = total_size;
        }

        let mut file = match tokio::fs::File::create(&part_dest).await {
            Ok(f) => f,
            Err(e) => {
                let mut st = state_arc.write().await;
                st.active = false;
                st.status = "error".to_string();
                st.error_message = Some(e.to_string());
                return;
            }
        };

        let mut stream = resp.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut last_instant = std::time::Instant::now();
        let mut last_bytes: u64 = 0;

        while let Some(chunk_res) = stream.next().await {
            if cancel_arc.load(Ordering::SeqCst) {
                let _ = tokio::fs::remove_file(&part_dest).await;
                let mut st = state_arc.write().await;
                st.active = false;
                st.status = "cancelled".to_string();
                return;
            }

            match chunk_res {
                Ok(chunk) => {
                    if let Err(e) = file.write_all(&chunk).await {
                        let mut st = state_arc.write().await;
                        st.active = false;
                        st.status = "error".to_string();
                        st.error_message = Some(e.to_string());
                        return;
                    }
                    downloaded += chunk.len() as u64;

                    if last_instant.elapsed() >= Duration::from_millis(500) {
                        let elapsed_s = last_instant.elapsed().as_secs_f64().max(0.001);
                        let bps = ((downloaded - last_bytes) as f64 / elapsed_s) as u64;
                        last_bytes = downloaded;
                        last_instant = std::time::Instant::now();

                        let pct = if total_size > 0 {
                            ((downloaded as f64 / total_size as f64) * 100.0).min(100.0) as u8
                        } else {
                            0
                        };

                        let mut st = state_arc.write().await;
                        st.bytes_downloaded = downloaded;
                        st.progress_pct = pct;
                        st.speed_bps = bps;
                    }
                }
                Err(e) => {
                    let mut st = state_arc.write().await;
                    st.active = false;
                    st.status = "error".to_string();
                    st.error_message = Some(e.to_string());
                    return;
                }
            }
        }

        let _ = file.flush().await;
        drop(file);

        if cancel_arc.load(Ordering::SeqCst) {
            let _ = tokio::fs::remove_file(&part_dest).await;
            let mut st = state_arc.write().await;
            st.active = false;
            st.status = "cancelled".to_string();
            return;
        }

        if let Err(e) = std::fs::rename(&part_dest, &final_dest) {
            let mut st = state_arc.write().await;
            st.active = false;
            st.status = "error".to_string();
            st.error_message = Some(e.to_string());
            return;
        }

        {
            let mut st = state_arc.write().await;
            st.active = false;
            st.bytes_downloaded = total_size;
            st.progress_pct = 100;
            st.status = "completed".to_string();
        }

        let _ = events_tx.send(leadfinder_core::CoreEvent::CalibrationUpdated { capacity_per_day: None });
    });

    Json(serde_json::json!({ "status": "started", "filename": req.filename })).into_response()
}

async fn handle_download_model_status(State(ctx): State<Arc<PipelineContext>>) -> Response {
    let st = ctx.model_download_state.read().await.clone();
    Json(st).into_response()
}

async fn handle_download_model_cancel(State(ctx): State<Arc<PipelineContext>>) -> Response {
    ctx.model_download_cancel.store(true, Ordering::SeqCst);
    Json(serde_json::json!({ "status": "cancelling" })).into_response()
}

async fn handle_delete_model(
    State(ctx): State<Arc<PipelineContext>>,
    Path((kind, filename)): Path<(String, String)>,
) -> Response {
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Invalid filename" }))).into_response();
    }
    let sub = if kind == "whisper" { "whisper" } else { "llm" };
    let path = ctx.paths.models_dir.join(sub).join(&filename);
    if path.is_file() {
        if let Err(e) = std::fs::remove_file(&path) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    }
    Json(serde_json::json!({ "deleted": true, "filename": filename })).into_response()
}

async fn handle_scan_models_folder(State(ctx): State<Arc<PipelineContext>>) -> Response {
    let _ = std::fs::create_dir_all(ctx.paths.models_dir.join("llm"));
    let _ = std::fs::create_dir_all(ctx.paths.models_dir.join("whisper"));
    handle_list_models(State(ctx)).await
}

async fn handle_scan_external_ai_servers(
    State(_ctx): State<Arc<PipelineContext>>,
    Json(req): Json<ScanExternalAiServersRequest>,
) -> Response {
    let mut servers: Vec<leadfinder_core::ExternalAiServerStatus> = Vec::new();
    let client = reqwest::Client::builder().timeout(Duration::from_millis(2000)).build().unwrap_or_default();

    let t0 = std::time::Instant::now();
    let ollama_res = client.get("http://127.0.0.1:11434/api/tags").send().await;
    let mut ollama_status = leadfinder_core::ExternalAiServerStatus {
        server_type: "ollama".to_string(),
        endpoint_url: "http://localhost:11434".to_string(),
        is_online: false,
        latency_ms: None,
        models: Vec::new(),
    };
    if let Ok(resp) = ollama_res {
        if resp.status().is_success() {
            ollama_status.is_online = true;
            ollama_status.latency_ms = Some(t0.elapsed().as_millis() as u64);
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(arr) = body.get("models").and_then(|v| v.as_array()) {
                    for m in arr {
                        let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let size = m.get("size").and_then(|v| v.as_u64());
                        let param = m.get("details").and_then(|d| d.get("parameter_size")).and_then(|v| v.as_str()).map(|s| s.to_string());
                        let quant = m.get("details").and_then(|d| d.get("quantization_level")).and_then(|v| v.as_str()).map(|s| s.to_string());
                        if !name.is_empty() {
                            ollama_status.models.push(leadfinder_core::ExternalAiServerModel {
                                id: format!("ollama_{name}"),
                                name,
                                parameter_size: param,
                                quantization: quant,
                                size_bytes: size,
                            });
                        }
                    }
                }
            }
        }
    }
    servers.push(ollama_status);

    let t1 = std::time::Instant::now();
    let lm_res = client.get("http://127.0.0.1:1234/v1/models").send().await;
    let mut lm_status = leadfinder_core::ExternalAiServerStatus {
        server_type: "lmstudio".to_string(),
        endpoint_url: "http://localhost:1234".to_string(),
        is_online: false,
        latency_ms: None,
        models: Vec::new(),
    };
    if let Ok(resp) = lm_res {
        if resp.status().is_success() {
            lm_status.is_online = true;
            lm_status.latency_ms = Some(t1.elapsed().as_millis() as u64);
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(arr) = body.get("data").and_then(|v| v.as_array()) {
                    for m in arr {
                        let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        if !id.is_empty() {
                            lm_status.models.push(leadfinder_core::ExternalAiServerModel {
                                id: format!("lmstudio_{id}"),
                                name: id,
                                parameter_size: None,
                                quantization: None,
                                size_bytes: None,
                            });
                        }
                    }
                }
            }
        }
    }
    servers.push(lm_status);

    if let Some(custom_url) = req.custom_url.filter(|s| !s.is_empty()) {
        let t2 = std::time::Instant::now();
        let target_url = if custom_url.ends_with("/models") {
            custom_url.clone()
        } else if custom_url.ends_with("/v1") {
            format!("{custom_url}/models")
        } else {
            format!("{}/v1/models", custom_url.trim_end_matches('/'))
        };

        let custom_res = client.get(&target_url).send().await;
        let mut custom_status = leadfinder_core::ExternalAiServerStatus {
            server_type: "custom".to_string(),
            endpoint_url: custom_url,
            is_online: false,
            latency_ms: None,
            models: Vec::new(),
        };
        if let Ok(resp) = custom_res {
            if resp.status().is_success() {
                custom_status.is_online = true;
                custom_status.latency_ms = Some(t2.elapsed().as_millis() as u64);
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if let Some(arr) = body.get("data").and_then(|v| v.as_array()) {
                        for m in arr {
                            let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            if !id.is_empty() {
                                custom_status.models.push(leadfinder_core::ExternalAiServerModel {
                                    id: format!("custom_{id}"),
                                    name: id,
                                    parameter_size: None,
                                    quantization: None,
                                    size_bytes: None,
                                });
                            }
                        }
                    }
                }
            }
        }
        servers.push(custom_status);
    }

    Json(serde_json::json!({ "servers": servers })).into_response()
}

async fn handle_get_model_roles(State(ctx): State<Arc<PipelineContext>>) -> Response {
    if let Ok(Some(roles_val)) = ctx.db.get_setting("model_roles").await {
        if let Ok(cfg) = serde_json::from_value::<leadfinder_core::ModelHubRolesConfig>(roles_val) {
            return Json(cfg).into_response();
        }
    }

    let prof = ctx.db.get_profile().await.ok().flatten();
    let eff_asr = prof.as_ref().and_then(|p| p.chosen_json.get("effective")).and_then(|e| e.get("asr_model")).and_then(|v| v.as_str()).unwrap_or("large-v3-turbo");
    let eff_llm_prov = prof.as_ref().and_then(|p| p.chosen_json.get("effective")).and_then(|e| e.get("llm_provider")).and_then(|v| v.as_str()).unwrap_or("local");
    let eff_llm_model = prof.as_ref().and_then(|p| p.chosen_json.get("effective")).and_then(|e| e.get("llm_model")).and_then(|v| v.as_str()).unwrap_or("BgGPT-Gemma-3-4B-IT-Q4_K_M.gguf");

    let mut roles = std::collections::HashMap::new();
    roles.insert("asr".to_string(), leadfinder_core::ModelRoleConfig {
        role: "asr".to_string(),
        provider: "whisper".to_string(),
        model_name: eff_asr.to_string(),
        custom_url: None,
        temperature: None,
    });
    roles.insert("claim_extraction".to_string(), leadfinder_core::ModelRoleConfig {
        role: "claim_extraction".to_string(),
        provider: eff_llm_prov.to_string(),
        model_name: eff_llm_model.to_string(),
        custom_url: None,
        temperature: Some(0.0),
    });
    roles.insert("fact_verification".to_string(), leadfinder_core::ModelRoleConfig {
        role: "fact_verification".to_string(),
        provider: eff_llm_prov.to_string(),
        model_name: eff_llm_model.to_string(),
        custom_url: None,
        temperature: Some(0.0),
    });
    roles.insert("osint_dossier".to_string(), leadfinder_core::ModelRoleConfig {
        role: "osint_dossier".to_string(),
        provider: "gemini".to_string(),
        model_name: "gemini-2.5-flash".to_string(),
        custom_url: None,
        temperature: Some(0.2),
    });
    roles.insert("ocr".to_string(), leadfinder_core::ModelRoleConfig {
        role: "ocr".to_string(),
        provider: "tesseract".to_string(),
        model_name: "bul+eng".to_string(),
        custom_url: None,
        temperature: None,
    });

    let defaults = leadfinder_core::ModelHubRolesConfig {
        roles,
        consensus: leadfinder_core::ConsensusSettings::default(),
    };

    Json(defaults).into_response()
}

async fn handle_update_model_roles(
    State(ctx): State<Arc<PipelineContext>>,
    Json(cfg): Json<leadfinder_core::ModelHubRolesConfig>,
) -> Response {
    let val = serde_json::to_value(&cfg).unwrap_or(serde_json::Value::Null);
    if let Err(e) = ctx.db.set_setting("model_roles", &val).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    Json(serde_json::json!({ "saved": true })).into_response()
}

async fn handle_test_model(
    State(ctx): State<Arc<PipelineContext>>,
    Json(req): Json<leadfinder_core::ModelTestRequest>,
) -> Response {
    let start = std::time::Instant::now();
    let app_cfg = ctx.config.read().await.clone();

    let llm_provider: Arc<dyn leadfinder_core::Llm> = match req.provider.as_str() {
        "gemini" => {
            let key = leadfinder_core::resolve_secret("GEMINI_API_KEY");
            Arc::new(leadfinder_providers::GeminiLlm::new(
                &req.model,
                key,
                true,
                Duration::from_secs(45),
            ))
        }
        "ollama" => {
            let raw_url = req.custom_url.as_deref().unwrap_or("http://localhost:11434");
            let norm_url = if raw_url.ends_with("/v1") {
                raw_url.to_string()
            } else {
                format!("{}/v1", raw_url.trim_end_matches('/'))
            };
            Arc::new(leadfinder_providers::OpenAiCompatLlm::new(
                "ollama",
                norm_url,
                &req.model,
                None,
                Duration::from_secs(45),
            ))
        }
        "lmstudio" => {
            let raw_url = req.custom_url.as_deref().unwrap_or("http://localhost:1234");
            let norm_url = if raw_url.ends_with("/v1") {
                raw_url.to_string()
            } else {
                format!("{}/v1", raw_url.trim_end_matches('/'))
            };
            Arc::new(leadfinder_providers::OpenAiCompatLlm::new(
                "lmstudio",
                norm_url,
                &req.model,
                None,
                Duration::from_secs(45),
            ))
        }
        "heuristics" => {
            Arc::new(leadfinder_providers::HeuristicLlm::new())
        }
        _ => {
            let model_rel = format!("llm/{}", req.model);
            let resolved = ctx
                .paths
                .resolve_model_path(&model_rel)
                .or_else(|| ctx.paths.resolve_model_path(&req.model))
                .unwrap_or_else(|| ctx.paths.models_dir.join("llm").join(&req.model));
            let llama_bin = ctx
                .paths
                .resolve_binary("llama-server")
                .unwrap_or_else(|| ctx.paths.tools_dir.join("llama-server.exe"));
            Arc::new(
                leadfinder_providers::OpenAiCompatLlm::new(
                    "local",
                    &app_cfg.providers.local_llm_url,
                    &req.model,
                    None,
                    Duration::from_secs(60),
                )
                .with_local_autolaunch(leadfinder_providers::LocalLlamaLaunchSpec {
                    llama_bin,
                    model_path: resolved,
                    use_gpu: true,
                    context_size: 4096,
                }),
            )
        }
    };

    let llm_req = leadfinder_core::LlmJsonRequest {
        system_prompt: req.system_prompt.unwrap_or_else(|| "You are an AI assistant. Return strictly valid JSON with an answer field.".to_string()),
        user_prompt: req.prompt,
        json_schema: None,
        temperature: 0.1,
        max_tokens: 1024,
    };

    match llm_provider.complete_json(&llm_req).await {
        Ok(resp) => {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            let elapsed_s = (elapsed_ms as f64 / 1000.0).max(0.001);
            let tokens_per_second = (resp.tokens_out as f64 / elapsed_s * 10.0).round() / 10.0;
            Json(leadfinder_core::ModelTestResponse {
                success: true,
                elapsed_ms,
                tokens_in: resp.tokens_in,
                tokens_out: resp.tokens_out,
                tokens_per_second,
                raw_text: resp.raw_text,
                parsed_json: Some(resp.parsed_json),
                error: None,
            })
            .into_response()
        }
        Err(e) => {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            Json(leadfinder_core::ModelTestResponse {
                success: false,
                elapsed_ms,
                tokens_in: 0,
                tokens_out: 0,
                tokens_per_second: 0.0,
                raw_text: String::new(),
                parsed_json: None,
                error: Some(e.to_string()),
            })
            .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_blocks_use_exact_financial_and_legal_facts_without_boilerplate() {
        let audit = leadfinder_providers::AegisTemporalAudit {
            target: "Андрей Гюров".to_string(),
            snapshots_checked: 5,
            oldest_capture: Some("2021-11-14".to_string()),
            newest_capture: Some("2024-06-27".to_string()),
            historical_snapshots: Vec::new(),
            internet_archive_docs: Vec::new(),
            wiki_bio: Some(leadfinder_providers::WikiBioExtract {
                title: "Андрей Гюров".to_string(),
                language: "bg".to_string(),
                extract: "Андрей Атанасов Гюров е български икономист и политик, роден на 31 декември 1975 г. в Гоце Делчев. На 26 юли 2023 г. Народното събрание го избира за подуправител на БНБ.\n== Казус с КПКОНПИ и ВАС ==\nПрез юни 2024 г. КПК (бивша КПКОНПИ) постановява решение за несъвместимост заради дялово участие в Йонтех Инженеринг ООД (ЕИК 201567890). По казуса е образувано дело пред ВАС, който отправя преюдициално запитване до Съда на Европейския съюз (СЕС).".to_string(),
                revisions: Vec::new(),
            }),
            scrubbed_or_altered_risk: "Ниска вероятност за заличаване".to_string(),
            provenance_notes: Vec::new(),
        };

        let docs = vec![
            ScrapedOsintDoc {
                bucket: "fin".to_string(),
                title: "Справка за Йонтех Инженеринг ООД".to_string(),
                url: "https://papagal.bg/eik/201567890/abc".to_string(),
                text: "Андрей Гюров фигурира като съдружник в Йонтех Инженеринг ООД с ЕИК 201567890 и капитал 5000 лв., подадена е имуществена декларация пред КПКОНПИ.".to_string(),
            },
            ScrapedOsintDoc {
                bucket: "legal".to_string(),
                title: "ВАС спря производството по делото на Андрей Гюров и сезира СЕС".to_string(),
                url: "https://news.lex.bg/vas-gyurov-ses/".to_string(),
                text: "Върховният административен съд (ВАС) образува адм. дело № 7812/2024 г. срещу решението на Управителния съвет на БНБ и акта на КПКОНПИ и отправи запитване до СЕС.".to_string(),
            },
        ];

        let corpus = format!(
            "{}\n{}\n{}",
            audit.wiki_bio.as_ref().unwrap().extract,
            docs[0].text,
            docs[1].text
        );
        let pivots = leadfinder_providers::extract_entity_pivots(&corpus, "Андрей Гюров");
        let eiks = leadfinder_providers::extract_eik_numbers(&corpus);

        let b1 = generate_fallback_block1("Андрей Гюров", "person", &audit, &pivots, &eiks, &docs);
        let b2 = generate_fallback_block2("Андрей Гюров", "person", &audit, &[], &docs);

        assert!(b1.contains("Йонтех Инженеринг ООД"), "Expected company name in Block 1: {}", b1);
        assert!(b1.contains("201567890"), "Expected EIK in Block 1: {}", b1);
        assert!(!b1.contains("При автоматизирания мониторинг"), "Must not contain generic boilerplate");
        assert!(!b1.contains("военна"), "Must not contain Rumen Radev hardcoded military text");

        assert!(b2.contains("ВАС"), "Expected court reference in Block 2: {}", b2);
        assert!(b2.contains("СЕС"), "Expected ECJ reference in Block 2: {}", b2);
        assert!(b2.contains("7812/2024"), "Expected exact case number in Block 2: {}", b2);
    }

    #[test]
    fn test_fallback_blocks_support_infrastructure_and_building_types() {
        let audit = leadfinder_providers::AegisTemporalAudit {
            target: "Автомагистрала Хемус".to_string(),
            snapshots_checked: 0,
            oldest_capture: None,
            newest_capture: None,
            historical_snapshots: Vec::new(),
            internet_archive_docs: Vec::new(),
            wiki_bio: None,
            scrubbed_or_altered_risk: "Не са установени данни за целенасочено премахване.".to_string(),
            provenance_notes: Vec::new(),
        };
        let docs = vec![ScrapedOsintDoc {
            bucket: "fin".to_string(),
            title: "Договори за строителство на АМ Хемус".to_string(),
            url: "https://api.bg/hemus-contracts".to_string(),
            text: "Агенция Пътна инфраструктура възложи инхаус договори на Автомагистрали ЕАД на стойност 1.3 млрд. лв. с надзор от ДНСК.".to_string(),
        }];
        let pivots = leadfinder_providers::extract_entity_pivots(&docs[0].text, "Автомагистрала Хемус");
        let b1 = generate_fallback_block1("Автомагистрала Хемус", "infrastructure", &audit, &pivots, &[], &docs);
        assert!(b1.contains("ГЛАВА 2: ФИНАНСИРАНЕ, ОБЩЕСТВЕНИ ПОРЪЧКИ, ДОГОВОРИ И СТОЙНОСТ"));
        assert!(b1.contains("Автомагистрали ЕАД"));
        assert!(b1.contains("1.3 млрд. лв."));
    }

    #[test]
    fn test_ensure_concrete_facts_preserved_appends_omitted_facts_to_llm_prose() {
        let llm_summary_prose = "# ГЛАВА 1: БИОГРАФИЧНИ ДАННИ\nАндрей Гюров е български икономист и народен представител.\n\n# ГЛАВА 2: ТЪРГОВСКИ РЕГИСТЪР\nУчаствал е в частния сектор преди влизането си в политиката.".to_string();
        let pivots = vec![leadfinder_core::EntityPivotRef {
            entity_name: "Йонтех Инженеринг ООД".to_string(),
            entity_kind: "ООД".to_string(),
            relevance: 0.9,
            registry_status: Some("ЕИК 201567890".to_string()),
        }];
        let eiks = vec!["201567890".to_string()];
        let fin_facts = vec![
            "Според Търговския регистър е вписан като съдружник в Йонтех Инженеринг ООД с ЕИК 201567890. ([Справка](https://papagal.bg/eik/201567890/b2c1))".to_string()
        ];
        let enriched_b1 = ensure_concrete_facts_preserved_block1(
            llm_summary_prose,
            &pivots,
            &eiks,
            &fin_facts,
            &[],
        );
        assert!(enriched_b1.contains("201567890"));
        assert!(enriched_b1.contains("Йонтех Инженеринг ООД"));
        assert!(enriched_b1.contains("papagal.bg/eik/201567890"));

        let llm_legal_prose = "# ГЛАВА 5: СЪДЕБНИ ПРОИЗВОДСТВА\nРазглеждани са административни спорове.".to_string();
        let legal_facts = vec![
            "Върховният административен съд образува адм. дело № 7812/2024 г. и отправи преюдициално запитване до СЕС. ([ВАС](https://www.sac.government.bg/news))".to_string()
        ];
        let enriched_b2 = ensure_concrete_facts_preserved_block2(llm_legal_prose, &[], &legal_facts);
        assert!(enriched_b2.contains("7812/2024"));
        assert!(enriched_b2.contains("СЕС"));
    }

    #[test]
    fn test_international_subject_extracts_eponymous_companies_dollars_and_impeachment_facts() {
        let wiki_txt = "== Въведение ==\nДоналд Джон Тръмп е американски бизнесмен и 45-и и 47-и президент на САЩ.\n\n== Бизнес кариера ==\nТръмп е президент и главен изпълнителен директор на „Тръмп Организейшън“ и основател на „Тръмп Ентертеймънт Ризортс“. Към 2007 г. годишният доход на Тръмп възлиза на 32 милиона долара, а нетната стойност на богатството му към 2017 г. възлиза на $3,7 милиарда. На 13 януари 2021 г., с обвинение за подбуждане на метеж, Камарата на представителите гласува за втори път негов импийчмънт, след което е оправдан от Сената.";
        let audit = leadfinder_providers::AegisTemporalAudit {
            target: "Доналд Тръмп".to_string(),
            snapshots_checked: 0,
            oldest_capture: None,
            newest_capture: None,
            historical_snapshots: Vec::new(),
            internet_archive_docs: Vec::new(),
            wiki_bio: Some(leadfinder_providers::WikiBioExtract {
                title: "Доналд Тръмп".to_string(),
                language: "bg".to_string(),
                extract: wiki_txt.to_string(),
                revisions: Vec::new(),
            }),
            scrubbed_or_altered_risk: "Не са установени данни за премахване.".to_string(),
            provenance_notes: Vec::new(),
        };
        let pivots = leadfinder_providers::extract_entity_pivots(wiki_txt, "Доналд Тръмп");
        let b1 = generate_fallback_block1("Доналд Тръмп", "person", &audit, &pivots, &[], &[]);
        let b2 = generate_fallback_block2("Доналд Тръмп", "person", &audit, &[], &[]);
        assert!(b1.contains("Тръмп Организейшън"), "Expected eponymous company in Chapter 2: {}", b1);
        assert!(b1.contains("32 милиона долара") || b1.contains("$3,7 милиарда"), "Expected USD financial figures in Chapter 2: {}", b1);
        assert!(b2.contains("импийчмънт"), "Expected impeachment ruling in Chapter 5: {}", b2);
    }
}

