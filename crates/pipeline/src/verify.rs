use std::collections::HashSet;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use leadfinder_core::{
    AegisDeepfakeResult, AppConfig, Claim, CoreEvent, Evidence, FactCheckArchive, LlmJsonRequest,
    Search, SearchHit, SearchQuery, Stage, VideoState,
};
use leadfinder_providers::{
    global_lexicon, ConfiguredFactCheckArchive, DynamicNewsStream, LlmChainRunner,
    PassageExtractorAndRanker, RankedPassage, WebAndPrimarySearch,
};
use crate::engine::PipelineContext;

const DEFAULT_VERIFY_SYSTEM_PROMPT: &str = "You are a retrieval-grounded Bulgarian fact-checking assistant. Compare the claim strictly to the provided passages. Return strictly valid JSON.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationStageReport {
    pub video_id: String,
    pub claim_id: i64,
    pub claim_text: String,
    pub verdict: String,
    pub label: String,
    pub reasoning_bg: String,
    pub evidence_strength: f64,
    pub passages_count: usize,
    pub stored_evidence_count: usize,
    pub forced_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawStatementCheck {
    pub statement_bg: String,
    pub status: String,
    pub check_bg: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawVerificationLlmOutput {
    pub claim_id: i64,
    pub verdict: String,
    pub label: String,
    pub evidence: Vec<RawEvidenceRef>,
    #[serde(default)]
    pub statement_checks: Vec<RawStatementCheck>,
    pub reasoning_bg: String,
    pub evidence_strength: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvidenceRef {
    pub passage_id: String,
    pub stance: String,
}

pub fn verification_output_json_schema() -> Value {
    json!({
        "type": "object",
        "required": ["claim_id", "verdict", "label", "evidence", "statement_checks", "reasoning_bg", "evidence_strength"],
        "additionalProperties": false,
        "properties": {
            "claim_id": { "type": "integer" },
            "verdict": {
                "type": "string",
                "enum": ["SUPPORTED", "CONTRADICTED", "UNVERIFIABLE", "NOT_CHECKABLE", "ALREADY_CHECKED"]
            },
            "label": {
                "type": "string",
                "enum": ["false_claim", "misleading", "missing_context", "satire_or_opinion", "supported", "unverifiable", "unknown"]
            },
            "evidence": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["passage_id", "stance"],
                    "additionalProperties": false,
                    "properties": {
                        "passage_id": { "type": "string", "minLength": 2 },
                        "stance": { "type": "string", "enum": ["supports", "contradicts", "neutral"] }
                    }
                }
            },
            "statement_checks": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["statement_bg", "status", "check_bg"],
                    "additionalProperties": false,
                    "properties": {
                        "statement_bg": { "type": "string", "minLength": 2 },
                        "status": { "type": "string", "enum": ["SUPPORTED", "CONTRADICTED", "UNVERIFIABLE"] },
                        "check_bg": { "type": "string", "minLength": 4 }
                    }
                }
            },
            "reasoning_bg": { "type": "string", "minLength": 5 },
            "evidence_strength": { "type": "number", "minimum": 0.0, "maximum": 1.0 }
        }
    })
}

pub fn sanitize_label(label: &str) -> String {
    let low = label.trim().to_lowercase();
    match low.as_str() {
        "false_claim" => "false_claim".to_string(),
        "misleading" => "misleading".to_string(),
        "missing_context" => "missing_context".to_string(),
        "satire_or_opinion" => "satire_or_opinion".to_string(),
        "disinformation" | "fake_news" => "false_claim".to_string(),
        "supported" | "verified" => "supported".to_string(),
        _ => "unverifiable".to_string(),
    }
}

pub fn distill_claim_keywords(claim_text: &str, actor: Option<&str>, context_words: &[String]) -> String {
    static STOP_WORDS: &[&str] = &[
        "и", "в", "на", "с", "със", "че", "да", "не", "се", "си", "ми", "му", "ни", "ви", "те", "тя", "той",
        "то", "от", "за", "по", "до", "при", "след", "преди", "като", "нали", "всъщност", "може", "би", "това",
        "този", "тази", "тези", "който", "която", "което", "които", "каза", "попитах", "стана", "беше", "съм",
        "сме", "сте", "са", "е", "а", "но", "или", "ли", "ще", "бяха", "вече", "дали", "защото", "всичко",
        "няма", "само", "тук", "там", "ме", "го", "я", "бил", "била", "било", "били", "както", "защо", "какво",
        "кога", "къде", "кой", "коя", "кое", "кои", "все", "още", "много", "малко", "дори", "вече", "едва",
        "пак", "така", "виж", "вижте"
    ];

    let mut tokens: Vec<String> = Vec::new();
    let mut seen = HashSet::new();

    if let Some(act) = actor {
        let act_trim = act.trim();
        let act_low = act_trim.to_lowercase();
        if !act_trim.is_empty()
            && !act_low.contains("неизвестен")
            && !act_low.contains("някой")
            && !act_low.contains("speaker")
            && !act_low.contains("хората")
            && !act_low.contains("граждани")
        {
            for part in act_trim.split_whitespace() {
                if seen.insert(part.to_lowercase()) {
                    tokens.push(part.to_string());
                }
            }
        }
    }

    for cw in context_words {
        let cw_trim = cw.trim();
        if !cw_trim.is_empty() && cw_trim.chars().count() >= 3 && seen.insert(cw_trim.to_lowercase()) {
            tokens.push(cw_trim.to_string());
            if tokens.len() >= 3 {
                break;
            }
        }
    }

    let re_spaced_digits = regex::Regex::new(r"(\d)\s+(\d{3})\b").unwrap();
    let cleaned_claim = re_spaced_digits.replace_all(claim_text, "$1$2");

    for word in cleaned_claim.split(|c: char| !c.is_alphanumeric()) {
        let w = word.trim();
        if (w.chars().count() < 3 && !w.chars().any(|c| c.is_ascii_digit())) || w.chars().all(|c| c == '0') {
            continue;
        }
        let low = w.to_lowercase();
        if STOP_WORDS.contains(&low.as_str()) {
            continue;
        }
        if seen.insert(low) {
            tokens.push(w.to_string());
            if tokens.len() >= 7 {
                break;
            }
        }
    }

    if tokens.is_empty() {
        claim_text.chars().take(80).collect()
    } else {
        tokens.join(" ")
    }
}

pub fn enforce_verification_rules(
    claim: &Claim,
    raw_output: &RawVerificationLlmOutput,
    passages: &[RankedPassage],
    min_independent_news_outlets: usize,
) -> (String, String, Vec<Evidence>, String, f64, Option<String>) {
    if claim.claim_type == "opinion" {
        return (
            "NOT_CHECKABLE".to_string(),
            "satire_or_opinion".to_string(),
            Vec::new(),
            "Твърдението представлява субективно мнение или оценка и не подлежи на фактическа проверка.".to_string(),
            0.0,
            Some("opinion_not_checkable".to_string()),
        );
    }

    if passages.is_empty() {
        return (
            "UNVERIFIABLE".to_string(),
            "unverifiable".to_string(),
            Vec::new(),
            "Няма открити релевантни институционални пасажи или проверки на факти за това твърдение.".to_string(),
            0.0,
            Some("no_passages_retrieved".to_string()),
        );
    }

    let valid_ids: HashSet<&str> = passages.iter().map(|p| p.passage_id.as_str()).collect();
    let mut stored_evidence: Vec<Evidence> = Vec::new();
    let mut seen_pids = HashSet::new();

    for ev in &raw_output.evidence {
        if !valid_ids.contains(ev.passage_id.as_str()) {
            continue;
        }
        if !seen_pids.insert(ev.passage_id.clone()) {
            continue;
        }
        if let Some(p) = passages.iter().find(|x| x.passage_id == ev.passage_id) {
            let stance = match ev.stance.to_lowercase().as_str() {
                "supports" => "supports".to_string(),
                "contradicts" => "contradicts".to_string(),
                _ => "neutral".to_string(),
            };
            stored_evidence.push(Evidence {
                id: 0,
                claim_id: claim.id,
                passage_id: p.passage_id.clone(),
                source_url: p.source_url.clone(),
                source_name: p.source_name.clone(),
                source_kind: p.source_kind.clone(),
                published_at: p.published_at.clone(),
                passage: p.text.clone(),
                stance: Some(stance),
                retrieved_at: chrono::Utc::now().to_rfc3339(),
            });
        }
    }

    let mut verdict = raw_output.verdict.to_uppercase();
    let mut label = sanitize_label(&raw_output.label);
    let mut reasoning = raw_output.reasoning_bg.trim().to_string();
    let mut strength = raw_output.evidence_strength.clamp(0.0, 1.0);
    let mut forced_reason = None;

    if verdict == "ALREADY_CHECKED" {
        let has_factcheck_evidence = stored_evidence.iter().any(|e| e.source_kind == "factcheck");
        if !has_factcheck_evidence {
            verdict = "UNVERIFIABLE".to_string();
            label = "unverifiable".to_string();
            strength = 0.0;
            forced_reason = Some("already_checked_missing_factcheck_link".to_string());
        }
    } else if verdict == "SUPPORTED" || verdict == "CONTRADICTED" {
        let has_primary_or_fc = stored_evidence
            .iter()
            .any(|e| e.source_kind == "primary" || e.source_kind == "factcheck");

        let news_domains: HashSet<String> = stored_evidence
            .iter()
            .filter(|e| e.source_kind == "news")
            .filter_map(|e| url::Url::parse(&e.source_url).ok()?.host_str().map(|h| h.to_lowercase()))
            .collect();

        let has_valid_url = stored_evidence.iter().any(|e| e.source_url.starts_with("http"));
        if !has_valid_url {
            verdict = "UNVERIFIABLE".to_string();
            label = "unverifiable".to_string();
            strength = 0.0;
            forced_reason = Some("missing_external_source_url".to_string());
            reasoning = format!("{reasoning} (Липсва проверен външен източник с пълен валиден уеб адрес.)");
        } else {
            let sufficient = has_primary_or_fc || news_domains.len() >= min_independent_news_outlets;
            if !sufficient {
                verdict = "UNVERIFIABLE".to_string();
                label = "unverifiable".to_string();
                strength = (strength * 0.4).min(0.35);
                forced_reason = Some("insufficient_authoritative_sources_forced_unverifiable".to_string());
                reasoning = format!("{reasoning} (Липсва минимално необходимият брой независими потвърждения съгласно правилата.)");
            } else {
                let claim_low = claim.text_bg.to_lowercase();
                let is_event = claim_low.contains("протест")
                    || claim_low.contains("стачк")
                    || claim_low.contains("демонстрац")
                    || claim_low.contains("сблъс")
                    || claim_low.contains("арест")
                    || claim_low.contains("безредиц")
                    || claim_low.contains("погром")
                    || claim_low.contains("бунт")
                    || claim_low.contains("митинг")
                    || claim_low.contains("ранен")
                    || claim_low.contains("убит");

                let r_low = reasoning.to_lowercase();
                let has_demographic_reasoning = r_low.contains("население")
                    || r_low.contains("жители")
                    || r_low.contains("преброяване")
                    || r_low.contains("демографи");

                if is_event && has_demographic_reasoning {
                    verdict = "UNVERIFIABLE".to_string();
                    label = "unverifiable".to_string();
                    strength = (strength * 0.3).min(0.25);
                    forced_reason = Some("demographic_fallacy_detected".to_string());
                    reasoning = format!("{reasoning} (Демографските данни за население не представляват доказателство за броя на участниците в протест или инцидент.)");
                } else if verdict == "CONTRADICTED" && !has_primary_or_fc {
                    let is_denial_dispute = r_low.contains("отрича")
                        || r_low.contains("отрече")
                        || r_low.contains("не е казвал")
                        || r_low.contains("лъже")
                        || r_low.contains("събеседник")
                        || r_low.contains("спор")
                        || r_low.contains("във видеото");
                    if is_denial_dispute {
                        verdict = "UNVERIFIABLE".to_string();
                        label = "unverifiable".to_string();
                        strength = (strength * 0.4).min(0.35);
                        forced_reason = Some("intra_video_dispute_unverifiable".to_string());
                        reasoning = format!("{reasoning} (Спорът или взаимното отричане между събеседници не представлява документално доказателство за опровержение. Изисква се първичен документ или верифициран фактчек.)");
                    }
                }
            }
        }
    }

    if stored_evidence.is_empty() && verdict != "NOT_CHECKABLE" {
        verdict = "UNVERIFIABLE".to_string();
        label = "unverifiable".to_string();
        strength = 0.0;
        forced_reason = Some("no_valid_passage_ids_forced_unverifiable".to_string());
    }

    if !raw_output.statement_checks.is_empty()
        && forced_reason.as_deref() != Some("demographic_fallacy_detected")
    {
        let downgrade_all = matches!(
            forced_reason.as_deref(),
            Some("missing_external_source_url")
                | Some("no_valid_passage_ids_forced_unverifiable")
                | Some("intra_video_dispute_unverifiable")
        );
        let mut lines = Vec::new();
        for sc in &raw_output.statement_checks {
            let st_txt = sc.statement_bg.trim().replace('\n', " ");
            let ch_txt = sc.check_bg.trim().replace('\n', " ");
            if st_txt.is_empty() && ch_txt.is_empty() {
                continue;
            }
            let tag = if downgrade_all {
                "[НЕПРОВЕРИМО]"
            } else {
                match sc.status.trim().to_uppercase().as_str() {
                    "SUPPORTED" => "[ПОТВЪРДЕНО]",
                    "CONTRADICTED" => "[ОПРОВЕРГАНО]",
                    _ => "[НЕПРОВЕРИМО]",
                }
            };
            lines.push(format!("{tag} {st_txt} :: {ch_txt}"));
        }
        if !lines.is_empty() {
            lines.push(format!("[ОБОБЩЕНИЕ] {reasoning}"));
            reasoning = lines.join("\n");
        }
    }

    (verdict, label, stored_evidence, reasoning, strength, forced_reason)
}

pub fn build_verify_user_prompt(
    claim: &Claim,
    passages: &[RankedPassage],
    date_window: Option<&str>,
    aegis_context: Option<&str>,
) -> String {
    let mut sections = Vec::new();
    sections.push(format!(
        "CLAIM_TO_VERIFY:\n- id: {}\n- text_bg: {}\n- claim_type: {}\n- speaker: {}\n- date_window: {}",
        claim.id,
        claim.text_bg,
        claim.claim_type,
        claim.speaker.as_deref().unwrap_or("unknown"),
        date_window.unwrap_or("unknown")
    ));

    if let Some(aegis) = aegis_context {
        if !aegis.is_empty() {
            sections.push(format!("AEGIS_DEEPFAKE_FORENSIC_INTELLIGENCE:\n{}", aegis));
        }
    }

    if !claim.attribution.is_empty() {
        let mut att_lines = Vec::new();
        for a in &claim.attribution {
            att_lines.push(format!(
                "- actor: {} | act: {} | target: {:?} | about: {}\n  asserted_part: {}\n  attributed_part: {}",
                a.actor,
                a.act,
                a.target,
                a.about,
                a.asserted_part.as_deref().unwrap_or(""),
                a.attributed_part.as_deref().unwrap_or("")
            ));
        }
        sections.push(format!("ATTRIBUTION_DETAILS:\n{}", att_lines.join("\n")));
    }

    let mut pass_lines = Vec::new();
    for p in passages {
        pass_lines.push(format!(
            "[{}] ({}) {} [pub: {}]\nИзточник URL: {}\n{}",
            p.passage_id,
            p.source_kind,
            p.source_name.as_deref().unwrap_or("Медия / Документ"),
            p.published_at.as_deref().unwrap_or("неизвестна дата"),
            p.source_url,
            p.text
        ));
    }
    sections.push(format!("RETRIEVED_PASSAGES:\n{}", pass_lines.join("\n\n")));

    sections.join("\n\n")
}

impl PipelineContext {
    pub async fn verify_claim_with_retrieval(
        &self,
        video_id: &str,
        claim_id: i64,
    ) -> Result<VerificationStageReport> {
        let claim = self
            .db
            .get_claims_for_video(video_id)
            .await?
            .into_iter()
            .find(|c| c.id == claim_id)
            .ok_or_else(|| anyhow::anyhow!("claim not found: {claim_id}"))?;
        let video = self
            .db
            .get_video(video_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("video not found: {video_id}"))?;

        let cfg: AppConfig = self.config.read().await.clone();
        let lex = global_lexicon(&self.paths, &cfg.normalisation.lexicon_config_file);

        if claim.claim_type == "opinion" {
            let dummy = RawVerificationLlmOutput {
                claim_id: claim.id,
                verdict: "NOT_CHECKABLE".to_string(),
                label: "satire_or_opinion".to_string(),
                evidence: Vec::new(),
                statement_checks: Vec::new(),
                reasoning_bg: "Субективно мнение или оценка.".to_string(),
                evidence_strength: 0.0,
            };
            let (verdict, label, evs, reasoning, strength, forced) = enforce_verification_rules(
                &claim,
                &dummy,
                &[],
                cfg.verification.min_independent_news_outlets,
            );
            self.db
                .update_claim_verification(claim.id, &verdict, &label, &reasoning, strength, &evs)
                .await?;
            self.check_and_mark_video_verified(video_id).await?;
            return Ok(VerificationStageReport {
                video_id: video_id.to_string(),
                claim_id: claim.id,
                claim_text: claim.text_bg,
                verdict,
                label,
                reasoning_bg: reasoning,
                evidence_strength: strength,
                passages_count: 0,
                stored_evidence_count: evs.len(),
                forced_reason: forced,
            });
        }

        let archive_ep = cfg.factcheck_archives.clone();
        let archive = ConfiguredFactCheckArchive::new(
            archive_ep.clone(),
            lex.clone(),
            self.guard.clone(),
            cfg.discovery.region.clone(),
        );

        let primary_entries =
            leadfinder_core::load_primary_sources(&self.paths.config_dir).unwrap_or_default();
        let search_engine = WebAndPrimarySearch::new(
            &cfg.providers.searxng_url,
            primary_entries.clone(),
            archive_ep.clone(),
            cfg.verification.news_domains.clone(),
            self.guard.clone(),
        );

        let ranker = PassageExtractorAndRanker::new(self.guard.clone(), lex.clone());
        let mut candidate_hits: Vec<(SearchHit, String)> = Vec::new();

        let video_people = self.db.get_people_for_video(video_id).await.unwrap_or_default();
        let mut context_terms: Vec<String> = Vec::new();
        for p in &video_people {
            if !p.canonical_name.is_empty() {
                context_terms.push(p.canonical_name.clone());
            }
        }
        if let Some(c) = &video.caption {
            for word in c.split_whitespace().take(5) {
                if word.chars().count() >= 4 {
                    context_terms.push(word.to_string());
                }
            }
        }
        for tag in &video.hashtags {
            let clean_tag = tag.trim_start_matches('#');
            if clean_tag.chars().count() >= 4 {
                context_terms.push(clean_tag.to_string());
            }
        }
        let actor = claim.speaker.as_deref().or_else(|| claim.attribution.first().map(|a| a.actor.as_str()));
        let distilled_q = distill_claim_keywords(&claim.text_bg, actor, &context_terms);

        if let Ok(fc_matches) = archive.find_fact_checks(&distilled_q).await {
            for m in fc_matches {
                let hit = SearchHit {
                    url: m.url.clone(),
                    title: m.title.clone(),
                    snippet: m.summary.clone(),
                    source_name: Some(m.archive_name.clone()),
                    source_kind: "factcheck".to_string(),
                    published_at: m.published_at.clone(),
                };
                let body = ranker.fetch_html_cached(&m.url).await.unwrap_or_default();
                let main_txt = ranker.extract_main_text(&body);
                candidate_hits.push((hit, main_txt));
            }
        }
        if candidate_hits.is_empty() {
            if let Ok(fc_matches) = archive.find_fact_checks(&claim.text_bg).await {
                for m in fc_matches {
                    let hit = SearchHit {
                        url: m.url.clone(),
                        title: m.title.clone(),
                        snippet: m.summary.clone(),
                        source_name: Some(m.archive_name.clone()),
                        source_kind: "factcheck".to_string(),
                        published_at: m.published_at.clone(),
                    };
                    let body = ranker.fetch_html_cached(&m.url).await.unwrap_or_default();
                    let main_txt = ranker.extract_main_text(&body);
                    candidate_hits.push((hit, main_txt));
                }
            }
        }

        let mut primary_domains: Vec<String> = Vec::new();
        for p in &primary_entries {
            primary_domains.push(p.domain.clone());
        }

        let primary_query = SearchQuery {
            query: distilled_q.clone(),
            restrict_domains: primary_domains,
            date_from: None,
            date_to: None,
            max_results: 6,
        };
        if let Ok(p_hits) = search_engine.search(&primary_query).await {
            for hit in p_hits {
                let body = ranker.fetch_html_cached(&hit.url).await.unwrap_or_default();
                let main_txt = ranker.extract_main_text(&body);
                candidate_hits.push((hit, main_txt));
            }
        }

        let web_query = SearchQuery {
            query: distilled_q.clone(),
            restrict_domains: Vec::new(),
            date_from: None,
            date_to: None,
            max_results: 6,
        };
        if let Ok(w_hits) = search_engine.search(&web_query).await {
            for hit in w_hits {
                let body = ranker.fetch_html_cached(&hit.url).await.unwrap_or_default();
                let main_txt = ranker.extract_main_text(&body);
                candidate_hits.push((hit, main_txt));
            }
        }

        let news_stream = DynamicNewsStream::new();
        if let Ok(n_hits) = news_stream.search_news_hits(&distilled_q, 6).await {
            for hit in n_hits {
                if !candidate_hits.iter().any(|(h, _)| h.url == hit.url) {
                    let body = ranker.fetch_html_cached(&hit.url).await.unwrap_or_default();
                    let main_txt = ranker.extract_main_text(&body);
                    candidate_hits.push((hit, main_txt));
                }
            }
        }
        if let Some(act) = actor {
            let actor_q = format!("{act} {distilled_q}");
            if let Ok(n_hits) = news_stream.search_news_hits(&actor_q, 4).await {
                for hit in n_hits {
                    if !candidate_hits.iter().any(|(h, _)| h.url == hit.url) {
                        let body = ranker.fetch_html_cached(&hit.url).await.unwrap_or_default();
                        let main_txt = ranker.extract_main_text(&body);
                        candidate_hits.push((hit, main_txt));
                    }
                }
            }
        }

        if candidate_hits.len() < 3 {
            let relaxed_q = distilled_q.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
            if !relaxed_q.is_empty() {
                let relaxed_query = SearchQuery {
                    query: format!("{} новини", relaxed_q),
                    restrict_domains: Vec::new(),
                    date_from: None,
                    date_to: None,
                    max_results: 4,
                };
                if let Ok(r_hits) = search_engine.search(&relaxed_query).await {
                    for hit in r_hits {
                        if !candidate_hits.iter().any(|(h, _)| h.url == hit.url) {
                            let body = ranker.fetch_html_cached(&hit.url).await.unwrap_or_default();
                            let main_txt = ranker.extract_main_text(&body);
                            candidate_hits.push((hit, main_txt));
                        }
                    }
                }
            }
        }

        let top_passages = ranker.rank_passages(
            &claim.text_bg,
            &candidate_hits,
            cfg.verification.passages_per_claim,
        );

        let prompt_path = self.paths.prompts_dir.join("verify_v1.txt");
        let system_prompt = std::fs::read_to_string(&prompt_path).unwrap_or_else(|_| {
            DEFAULT_VERIFY_SYSTEM_PROMPT.to_string()
        });

        let (verdict, label, stored_evidence, reasoning, strength, forced_reason) =
            if top_passages.is_empty() {
                let dummy = RawVerificationLlmOutput {
                    claim_id: claim.id,
                    verdict: "UNVERIFIABLE".to_string(),
                    label: "unverifiable".to_string(),
                    evidence: Vec::new(),
                    statement_checks: Vec::new(),
                    reasoning_bg: "Няма намерени пасажи".to_string(),
                    evidence_strength: 0.0,
                };
                enforce_verification_rules(
                    &claim,
                    &dummy,
                    &[],
                    cfg.verification.min_independent_news_outlets,
                )
            } else {
                let mut aegis_info = self.db.get_aegis_result(video_id).await.ok().flatten();
                if aegis_info.is_none() {
                    aegis_info = self.run_aegis_deepfake_detection(video_id).await.ok();
                }
                let aegis_context = aegis_info.as_ref().and_then(|df| {
                    let mut parts = Vec::new();
                    if df.audio.synthetic {
                        let target = df.audio.target_person.as_deref().unwrap_or("неустановена личност");
                        parts.push(format!("Аудио анализ: Засечен клониран/изкуствен глас ({:.0}% риск, имитиран глас: {}).", df.audio.score * 100.0, target));
                    }
                    if df.video.synthetic {
                        let target = df.video.target_person.as_deref().unwrap_or("неустановена личност");
                        parts.push(format!("Видео анализ: Засечено подменено лице / генерирано видео ({:.0}% риск, имитирано лице: {}).", df.video.score * 100.0, target));
                    }
                    if df.multimodal.av_sync_mismatch {
                        parts.push("Синхрон звук-устни: Открито разминаване между устните и говора (изкуствен дублаж).".to_string());
                    }
                    if !parts.is_empty() {
                        parts.push("ВНИМАНИЕ: Записът съдържа доказана цифрова манипулация от AEGIS. Ако твърдението приписва думи на публична личност чрез изкуствен глас или подменено лице, това представлява цифрова фалшификация!".to_string());
                        Some(parts.join("\n"))
                    } else if df.overall_verdict.eq_ignore_ascii_case("SUSPECTED_SYNTHETIC") || df.overall_verdict.eq_ignore_ascii_case("suspicious") {
                        Some(format!("AEGIS отчита съмнителни отклонения в записа (аудио риск {:.0}%, видео риск {:.0}%), но под прага за сигурна синтетика.", df.audio.score * 100.0, df.video.score * 100.0))
                    } else {
                        None
                    }
                });
                let mut user_prompt = build_verify_user_prompt(
                    &claim,
                    &top_passages,
                    video.posted_at.as_deref(),
                    aegis_context.as_deref(),
                );
                if let Ok(Some(notes)) = self.db.latest_claim_notes(claim.id).await {
                    user_prompt.push_str(&format!(
                        "\n\nREVIEWER_NOTES (насоки от потребителя; не са доказателство, не цитирай като източник):\n{}",
                        notes.trim()
                    ));
                }
                let runner = LlmChainRunner::from_role("fact_verification", &self.paths, &cfg, &self.db).await;
                let llm_req = LlmJsonRequest {
                    system_prompt,
                    user_prompt,
                    json_schema: Some(verification_output_json_schema()),
                    temperature: 0.0,
                    max_tokens: 2048,
                };
                let validated = runner.complete_validated(&llm_req, &self.db).await?;
                let raw_parsed: RawVerificationLlmOutput =
                    serde_json::from_value(validated.response.parsed_json)?;
                let (v, mut l, stored_evidence, mut r, mut s, forced_reason) = enforce_verification_rules(
                    &claim,
                    &raw_parsed,
                    &top_passages,
                    cfg.verification.min_independent_news_outlets,
                );

                if let Ok(Some(roles_val)) = self.db.get_setting("model_roles").await {
                    let consensus_enabled = roles_val
                        .get("consensus")
                        .and_then(|c| c.get("enabled"))
                        .and_then(|val| val.as_bool())
                        .unwrap_or(false);
                    if consensus_enabled {
                        let sec_runner = LlmChainRunner::from_role("consensus_secondary", &self.paths, &cfg, &self.db).await;
                        if let Ok(sec_val) = sec_runner.complete_validated(&llm_req, &self.db).await {
                            if let Ok(sec_parsed) = serde_json::from_value::<RawVerificationLlmOutput>(sec_val.response.parsed_json) {
                                let (sec_v, _sec_l, _sec_ev, sec_r, sec_s, _sec_fr) = enforce_verification_rules(
                                    &claim,
                                    &sec_parsed,
                                    &top_passages,
                                    cfg.verification.min_independent_news_outlets,
                                );
                                if v == sec_v {
                                    r = format!("{}\n\n[Мулти-моделен консенсус]: Единодушно съгласие между {} и {}.", r, validated.response.model, sec_val.response.model);
                                    s = (s + sec_s) / 2.0;
                                } else {
                                    r = format!("{}\n\n[Разминаване в консенсуса]: Модел 1 ({}) даде: {:?} ({:.0}%). Модел 2 ({}) даде: {:?} ({:.0}%).\nАргумент 2: {}",
                                        r, validated.response.model, v, s * 100.0,
                                        sec_val.response.model, sec_v, sec_s * 100.0, sec_r
                                    );
                                    l = format!("{l} [Двойна оценка]");
                                }
                            }
                        }
                    }
                }

                (v, l, stored_evidence, r, s, forced_reason)
            };

        self.db
            .update_claim_verification(
                claim.id,
                &verdict,
                &label,
                &reasoning,
                strength,
                &stored_evidence,
            )
            .await?;
        self.check_and_mark_video_verified(video_id).await?;

        Ok(VerificationStageReport {
            video_id: video_id.to_string(),
            claim_id: claim.id,
            claim_text: claim.text_bg,
            verdict,
            label,
            reasoning_bg: reasoning,
            evidence_strength: strength,
            passages_count: top_passages.len(),
            stored_evidence_count: stored_evidence.len(),
            forced_reason,
        })
    }

    pub async fn run_claims_job(&self, video_id: &str) -> Result<()> {
        let _ = self.run_aegis_deepfake_detection(video_id).await;
        self.extract_claims_for_video(video_id).await?;
        if let Ok(verify_job) = self.db.enqueue_job(Some(video_id), Stage::Verify).await {
            self.emit(CoreEvent::JobUpdated {
                job_id: verify_job.id,
                video_id: Some(video_id.to_string()),
                stage: verify_job.stage,
                status: verify_job.status,
            });
        }
        Ok(())
    }

    pub async fn run_verify_job(&self, video_id: &str) -> Result<()> {
        let _ = self.run_aegis_deepfake_detection(video_id).await;
        let claims = self.db.get_claims_for_video(video_id).await?;
        if claims.is_empty() {
            self.db
                .set_video_state(video_id, VideoState::Verified, None)
                .await?;
            if let Ok(Some(v)) = self.db.get_video(video_id).await {
                self.emit(CoreEvent::VideoUpdated {
                    video_id: video_id.to_string(),
                    state: v.state,
                    lane: v.lane,
                });
            }
            return Ok(());
        }
        for claim in claims {
            if claim.verdict.is_none() {
                self.verify_claim_with_retrieval(video_id, claim.id).await?;
            }
        }
        let _ = self.check_and_mark_video_verified(video_id).await;
        let _ = self.rank_all().await;
        if let Ok(Some(v)) = self.db.get_video(video_id).await {
            self.emit(CoreEvent::VideoUpdated {
                video_id: video_id.to_string(),
                state: v.state,
                lane: v.lane,
            });
        }
        Ok(())
    }

    pub async fn run_research_more_job(&self, video_id: &str) -> Result<()> {
        let _ = self.run_aegis_deepfake_detection(video_id).await;
        let mut claims = self.db.get_claims_for_video(video_id).await?;
        if claims.is_empty() {
            let _ = self.run_claims_job(video_id).await;
            claims = self.db.get_claims_for_video(video_id).await.unwrap_or_default();
        }
        for claim in &claims {
            self.verify_claim_with_retrieval(video_id, claim.id).await?;
        }

        let cfg: AppConfig = self.config.read().await.clone();
        let lex = global_lexicon(&self.paths, &cfg.normalisation.lexicon_config_file);
        let people_resolver =
            leadfinder_providers::PeopleResolver::new(cfg.normalisation.clone(), lex.clone(), self.guard.clone());

        let people = self.db.get_people_for_video(video_id).await?;
        for p in people {
            if p.wikidata_id.is_none() {
                if let Ok(Some(matched)) = people_resolver.lookup_wikidata(&p.canonical_name, &p.variants).await {
                    let summary = matched.description;
                    let _ = self
                        .db
                        .upsert_person_for_video(
                            video_id,
                            &p.canonical_name,
                            &p.variants,
                            p.role.as_deref(),
                            true,
                            Some(&matched.wikidata_id),
                            summary.as_deref(),
                            p.mentions.unwrap_or(1),
                            p.found_in.as_deref().unwrap_or("research_more"),
                            Some(&p.entity_type),
                        )
                        .await;
                }
            }
        }

        if let Some(video) = self.db.get_video(video_id).await? {
            if let Some(creator_id) = &video.creator_id {
                let filter = leadfinder_core::VideoFilter {
                    creator_id: Some(creator_id.clone()),
                    lane: None,
                    state: None,
                    verdict: None,
                    label: None,
                    search_query: None,
                    min_score: None,
                    min_views: None,
                    limit: Some(100),
                    offset: Some(0),
                };
                let creator_vids = self.db.list_videos(&filter).await?;
                let mut contradicted_claims = 0;
                let mut total_claims = 0;
                for cv in &creator_vids {
                    if let Ok(c_list) = self.db.get_claims_for_video(&cv.id).await {
                        for c in c_list {
                            total_claims += 1;
                            if c.verdict.as_deref() == Some("CONTRADICTED") {
                                contradicted_claims += 1;
                            }
                        }
                    }
                }
                if contradicted_claims >= 2 || (total_claims >= 3 && contradicted_claims > 0) {
                    let _ = self.db.set_creator_watchlist(creator_id, true).await;
                }
                let _ = crate::creator_risk::assess_creator_risk(self, creator_id, video.creator_display_name.as_deref(), false).await;
            }
        }

        let _ = self.check_and_mark_video_verified(video_id).await;
        let _ = self.rank_all().await;
        if let Some(v) = self.db.get_video(video_id).await? {
            self.emit(CoreEvent::VideoUpdated {
                video_id: video_id.to_string(),
                state: v.state,
                lane: v.lane,
            });
        }
        Ok(())
    }

    pub async fn run_aegis_deepfake_detection(&self, video_id: &str) -> Result<AegisDeepfakeResult> {
        let media_dir = self.media_root().join(video_id);
        let people = self.db.get_people_for_video(video_id).await.unwrap_or_default();
        let people_names: Vec<String> = people.into_iter().map(|p| p.canonical_name).collect();
        let people_json = serde_json::to_string(&people_names).unwrap_or_else(|_| "[]".to_string());

        let py_script = [
            self.paths.tools_dir.join("aegis_deepfake_detector.py"),
            self.paths.static_root.join("scripts").join("aegis_deepfake_detector.py"),
            self.paths.static_root.join("tools").join("aegis_deepfake_detector.py"),
            std::path::PathBuf::from("tools/aegis_deepfake_detector.py"),
            std::path::PathBuf::from("scripts/aegis_deepfake_detector.py"),
        ]
        .into_iter()
        .find(|p| p.is_file());

        let mut result = if let Some(script) = py_script {
            let mut cmd = tokio::process::Command::new("python");
            cmd.arg(&script)
                .arg("--media-dir")
                .arg(&media_dir)
                .arg("--people-json")
                .arg(&people_json)
                .arg("--json");
            #[cfg(windows)]
            {
                cmd.creation_flags(0x08000000);
            }
            let output = tokio::time::timeout(std::time::Duration::from_secs(25), cmd.output()).await??;
            if output.status.success() {
                serde_json::from_slice::<AegisDeepfakeResult>(&output.stdout)?
            } else {
                anyhow::bail!("Aegis detector script error: {}", String::from_utf8_lossy(&output.stderr));
            }
        } else {
            anyhow::bail!("Aegis detector script not found");
        };

        result.analyzed_at = chrono::Utc::now().to_rfc3339();
        self.db.save_aegis_result(video_id, &result).await?;

        if let Ok(Some(v)) = self.db.get_video(video_id).await {
            self.emit(CoreEvent::VideoUpdated {
                video_id: video_id.to_string(),
                state: v.state,
                lane: v.lane,
            });
        }

        Ok(result)
    }

    async fn check_and_mark_video_verified(&self, video_id: &str) -> Result<()> {
        let claims = self.db.get_claims_for_video(video_id).await?;
        let all_verified = claims.is_empty() || claims.iter().all(|c| c.verdict.is_some());
        if all_verified {
            self.db
                .set_video_state(video_id, VideoState::Verified, None)
                .await?;
            if let Ok(Some(v)) = self.db.get_video(video_id).await {
                self.emit(CoreEvent::VideoUpdated {
                    video_id: video_id.to_string(),
                    state: VideoState::Verified.as_str().to_string(),
                    lane: v.lane,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn redacting_claim_resets_verdict_and_exposes_latest_notes() {
        let dir = tempfile::tempdir().unwrap();
        let db = leadfinder_core::Database::connect(&dir.path().join("redact.db")).await.unwrap();
        let vref = leadfinder_core::VideoRef {
            id: "vid_redact".to_string(),
            url: "https://www.tiktok.com/@bg/video/7100000000000000002".to_string(),
            creator_id: Some("bg".to_string()),
            creator_handle: Some("bg".to_string()),
            creator_display_name: None,
            creator_follower_count: None,
            creator_region: Some("BG".to_string()),
            caption: None,
            hashtags: vec![],
            posted_at: None,
            views: None,
            likes: None,
            shares: None,
            comments: None,
            duration_s: None,
            sound_title: None,
            subtitle_urls: vec![],
            discovered_via: "paste".to_string(),
        };
        db.upsert_video_discovered(&vref, 0).await.unwrap();
        let claim = Claim {
            id: 0,
            video_id: "vid_redact".to_string(),
            text_bg: "Стар текст".to_string(),
            claim_type: "factual".to_string(),
            speaker: None,
            attribution: vec![],
            start_ms: None,
            end_ms: None,
            checkworthiness: None,
            verdict: None,
            label: None,
            reasoning_bg: None,
            evidence_strength: None,
            verified_at: None,
            cluster_id: None,
        };
        let saved = db.replace_claims_for_video("vid_redact", &[claim]).await.unwrap();
        let id = saved[0].id;
        db.update_claim_verification(id, "CONTRADICTED", "false_claim", "стар мотив", 0.9, &[]).await.unwrap();

        db.update_claim_text(id, "Нов текст").await.unwrap();
        db.save_claim_feedback(id, "vid_redact", "edited", None, Some("провери регистъра")).await.unwrap();

        let after = db.get_claims_for_video("vid_redact").await.unwrap();
        assert_eq!(after[0].text_bg, "Нов текст");
        assert!(after[0].verdict.is_none());
        assert!(after[0].reasoning_bg.is_none());
        assert_eq!(db.latest_claim_notes(id).await.unwrap().as_deref(), Some("провери регистъра"));
    }

    #[test]
    fn distill_claim_keywords_merges_spaced_numbers() {
        let q = distill_claim_keywords(
            "По данни на правителството над 550 000 души са излезли на протести във Франция",
            None,
            &[],
        );
        assert!(q.contains("550000"), "expected 550000 in query: {q}");
        assert!(!q.split_whitespace().any(|t| t == "000"));
    }

    #[test]
    fn demographic_fallacy_on_event_claim_forces_unverifiable() {
        let claim = Claim {
            id: 42,
            video_id: "7697938534824430870".to_string(),
            text_bg: "По данни на правителството, над 550 000 души са излезли по улиците, за да протестират срещу високите цени.".to_string(),
            claim_type: "factual".to_string(),
            speaker: None,
            attribution: vec![],
            start_ms: Some(12000),
            end_ms: Some(20000),
            checkworthiness: Some(0.9),
            verdict: None,
            label: None,
            reasoning_bg: None,
            evidence_strength: None,
            verified_at: None,
            cluster_id: None,
        };

        let passages = vec![
            RankedPassage {
                passage_id: "p1".to_string(),
                source_url: "https://factcheck.bg/sample".to_string(),
                source_name: Some("Factcheck.bg".to_string()),
                source_kind: "factcheck".to_string(),
                published_at: Some("2026-10-18".to_string()),
                text: "През 1800 г. населението на Париж е било около 500 000 жители.".to_string(),
                score: 1.2,
            },
        ];

        let raw = RawVerificationLlmOutput {
            claim_id: 42,
            verdict: "CONTRADICTED".to_string(),
            label: "false_claim".to_string(),
            evidence: vec![RawEvidenceRef {
                passage_id: "p1".to_string(),
                stance: "contradicts".to_string(),
            }],
            statement_checks: Vec::new(),
            reasoning_bg: "Опровергава се, тъй като според https://factcheck.bg/sample населението на Париж през 1800 г. е било 500 000 жители.".to_string(),
            evidence_strength: 0.9,
        };

        let (verdict, label, _, _, _, forced) = enforce_verification_rules(&claim, &raw, &passages, 2);
        assert_eq!(verdict, "UNVERIFIABLE");
        assert_eq!(label, "unverifiable");
        assert_eq!(forced.as_deref(), Some("demographic_fallacy_detected"));
    }

    #[test]
    fn statement_checks_format_per_statement_and_summary() {
        let claim = Claim {
            id: 43,
            video_id: "7697938534824430870".to_string(),
            text_bg: "1200 арестувани и 150 ранени полицаи при протестите във Франция.".to_string(),
            claim_type: "factual".to_string(),
            speaker: None,
            attribution: vec![],
            start_ms: Some(1000),
            end_ms: Some(8000),
            checkworthiness: Some(0.9),
            verdict: None,
            label: None,
            reasoning_bg: None,
            evidence_strength: None,
            verified_at: None,
            cluster_id: None,
        };
        let passages = vec![RankedPassage {
            passage_id: "p1".to_string(),
            source_url: "https://factcheck.bg/france-protests".to_string(),
            source_name: Some("Factcheck.bg".to_string()),
            source_kind: "factcheck".to_string(),
            published_at: Some("2026-10-18".to_string()),
            text: "Според МВР има 850 арестувани и 150 ранени полицаи.".to_string(),
            score: 1.4,
        }];
        let raw = RawVerificationLlmOutput {
            claim_id: 43,
            verdict: "CONTRADICTED".to_string(),
            label: "misleading".to_string(),
            evidence: vec![RawEvidenceRef {
                passage_id: "p1".to_string(),
                stance: "contradicts".to_string(),
            }],
            statement_checks: vec![
                RawStatementCheck {
                    statement_bg: "1200 арестувани".to_string(),
                    status: "CONTRADICTED".to_string(),
                    check_bg: "Пасаж p1 (https://factcheck.bg/france-protests) посочва 850 арестувани.".to_string(),
                },
                RawStatementCheck {
                    statement_bg: "150 ранени полицаи".to_string(),
                    status: "SUPPORTED".to_string(),
                    check_bg: "Пасаж p1 потвърждава 150 ранени полицаи.".to_string(),
                },
            ],
            reasoning_bg: "Според https://factcheck.bg/france-protests броят на арестуваните е завишен, макар броят на ранените полицаи да съвпада.".to_string(),
            evidence_strength: 0.85,
        };
        let (verdict, _, _, reasoning, _, _) = enforce_verification_rules(&claim, &raw, &passages, 2);
        assert_eq!(verdict, "CONTRADICTED");
        assert!(reasoning.contains("[ОПРОВЕРГАНО] 1200 арестувани ::"));
        assert!(reasoning.contains("[ПОТВЪРДЕНО] 150 ранени полицаи ::"));
        assert!(reasoning.contains("[ОБОБЩЕНИЕ] Според https://factcheck.bg/france-protests"));
    }
}
