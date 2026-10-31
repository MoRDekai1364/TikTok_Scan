# TikTok Lead Finder Memory & Architecture

## Stack & Architecture
- **Language**: Rust 2021 (backend/core/pipeline/providers), Python 3 (auxiliary scripts & ASR tools), SvelteKit / TypeScript (frontend UI)
- **Crates**:
  - `leadfinder-core`: Domain models, traits (`Transcriber`, `LlmProvider`, `VisionProber`), error types, event channels.
  - `leadfinder-pipeline`: Job engine, claim worker loop, stage orchestration (`SCREEN_CHEAP` -> `ACQUIRE` -> `CLAIMS` -> `VERIFY`).
  - `leadfinder-providers`: External integrations: WhisperCppTranscriber, CaptionFirstTranscriber, Ollama/OpenAI/Gemini LLMs, SearxngSearchProvider, WebScraper.
  - `leadfinder-probe`: Multimodal frame extraction, OCR probe, visual change detection.
  - `leadfinder-cli`: Command-line interface and background server runner.

## Key Directories & Files
- `crates/providers/src/transcriber.rs`: Hybrid speech transcriber with Whisper sentence segmentation, language detection routing, and Google AI speech integration.
- `crates/providers/src/sanctions.rs`: OpenSanctions live search & multi-jurisdiction sanctions/PEP cross-referencing engine (UN, US OFAC SDN, EU, UK OFSI, SECO, NAZK, Interpol).
- `crates/providers/src/news_stream.rs`: Dynamic real-time news stream engine aggregating Google News RSS, BTA, and BNR with deduplication.
- `crates/providers/src/web_archive.rs`: Temporal digital memory engine cross-referencing Wayback Machine (archive.org), Internet Archive documents, Wikipedia edit revisions, and digital scrubbing risk auditing.
- `crates/pipeline/src/creator_risk.rs`: Web-based author risk calculation engine using Factcheck.bg, BNT, Google search, and AEGIS forensic penalties.
- `crates/pipeline/src/verify.rs`: Verification pipeline with AEGIS deepfake and voice clone detection integrated into claim auditing.
- `crates/pipeline/src/server.rs`: REST API server with chunked 10k-word OSINT dossier synthesis, dynamic query formulation, entity pivoting, and live news streams.
- `crates/pipeline/src/engine.rs`: Pipeline worker loop, stage transitions, event emissions, and background continuous news stream monitoring.
- `tools/aegis_deepfake_detector.py` & `scripts/aegis_deepfake_detector.py`: Forensic audio/video synthetic detection engine (F0, vocoder cutoffs, 2D FFT, Laplacian variance, AV-sync).
- `release_stage/leadfinder.exe`: Staged release binary.

## Commands
- Build release: `cargo build --release -p leadfinder-cli`
- Run workspace tests: `cargo test --workspace`
- Run transcriber test: `python scripts/google_audio_transcribe.py --audio <path> --whisper-json <path>`

## Conventions & Constraints
- Strictly zero comments across all codebase files, configuration files, and git commits.
- All file links must use `file:///` format with forward slashes.
- Token shield output pipe discipline on verbose build and test outputs.

## Dynamic Living OSINT Architecture (5k - 10k Words)
- Multi-Block Chunked Synthesis: Never attempt to generate exhaustive dossiers in a single JSON schema pass. Break synthesis into sequential multi-chapter blocks (Chapters 1–3, 4–6, 7–9) at 2,500–3,500 words per block.
- Global Multi-Jurisdiction Sanctions: Cross-reference subjects against OpenSanctions and international registries (UN Security Council, US OFAC SDN / Magnitsky, EU Consolidated, UK OFSI, Swiss SECO, Ukraine НАЗК, Interpol Red Notices, World Bank) with local fallback.
- Live Media Stream Ingestion: Dynamically ingest real-time RSS feeds (Google News RSS query per entity, BTA, BNR) via hybrid polling (25-minute background loop + on-demand UI refresh).
- Recursive Entity Pivoting: Automatically detect commercial entity suffixes (ООД, ЕООД, АД, ЕАД, Холдинг, Консорциум) to spawn 2nd-degree sub-searches for beneficial ownership and corporate networks.
- Temporal Delta Tracking: Automatically compare current findings against previous scans and display a high-visibility delta banner ("Какво се промени от предходния скан").
- Temporal Digital Memory & Wayback Machine: Cross-reference subjects against Wayback Machine availability, CDX historical snapshots, Internet Archive documents/audio, and Wikipedia edit logs with scrubbing/alteration risk assessment.
- Primary Biographical Encyclopedic Depth: Ingest full structured Wikipedia extracts (50,000+ characters) to guarantee 90%+ focus on the subject's personal history, military/political appointments, asset declarations, and controversies, achieving 5,000–10,000 words per dossier.

## AEGIS Multi-Modal Forensics Invariants
- Pervasive Integration: AEGIS forensic metrics must feed claim verification prompts, creator risk scoring, and subject OSINT profiling.
- Strict Attribution Guardrail: Never attribute synthetic audio or video to a named public figure unless target attribution confidence is >= 80.0%.

## Dialectical Verification & Puncture Web Proof Invariants
- Anti-Fallacy Guardrail: Intra-video dialogues or disputes where Speaker 2 denies Speaker 1's statement ("не съм казвал това", "лъжеш") do not constitute evidence for CONTRADICTED or SUPPORTED. Without external primary documents (official registers, stenograms, voting records, or published fact-checks), verdicts must be UNVERIFIABLE.
- Puncture Proof Retrieval: Passage prompts must explicitly expose the full external source URL (`Източник URL: {}`), and LLM reasoning must cite the exact external URL. Unsupported verdicts lacking external HTTP URLs are automatically forced to UNVERIFIABLE.
- Auto-Trigger AEGIS: Deepfake forensic scanning is executed automatically upon video claims extraction and verify job enqueueing, ensuring zero-click forensic coverage.

## Entity Disambiguation & Deduplication (Google Contacts Engine)
- Ingestion Prevention: Pre-process captions, transcripts, and OCR with `sanitize_text_for_name_scanning` by replacing `#`, commas, colons, and semicolons with sentence-terminating boundaries (`. `), preventing adjacent hashtags or clause pronouns from merging into composite names.
- Multi-Type Decomposition: `EntityCleaner` filters Bulgarian stopwords, relative pronouns (`който`, `която`), honorifics (`господин`, `г-н`), and category keywords (`политика`, `българия`), resolving aliases and single surnames to canonical names.
- Interactive & Auto Deduplication: REST API (`/api/people/dedup/proposals`, `/api/people/dedup/apply`, `/api/people/dedup/auto`) exposes proposals with confidence scoring. The UI provides a Google Contacts style confirmation modal with auto-merge, compound splitting (`Пеевски Борисов` -> `Делян Пеевски` + `Бойко Борисов`), and tag cleaning (`Делянпеевски Политика` -> `Делян Пеевски` + `#политика`).
- TopBar Notification & Playback Center: Top bar notification pill with real-time badges opens a comprehensive center with tabs for questions/deduplication, system notifications, and video media player controls (play/pause, seek, volume, track info).

## Precision Verification, Evidence Highlighting & Dedup Ergonomics
- Anti-Demographic Fallacy: Wikipedia reclassified from `primary` to `reference` (0.8x score). Event claims (protests, riots, casualties) automatically discard general city census passages (`население`, `преброяване`, `жители`). Backend rule engine forces `UNVERIFIABLE` if reasoning commits demographic topic disconnection.
- Evidence Anchors & Keyword Marks: Evidence items have DOM IDs (`evidence-{id}`), glowing pulse transitions (`.evidence-pulse`), and passage keyword highlighting (`<mark class="matched-passage">`). Claim header has `🎯 Към доказателството` button to jump directly to the primary proof.
- RLHF Feedback Loop: 1-click feedback on claim cards (`👍 Вярна преценка`, `👎 Грешна преценка`, `⚠️ Несвързано доказателство`) stored in `claim_verification_feedback` table via `POST /api/claims/:id/feedback`.
- Dedup Ergonomics & Manual Merging: Proposal target names are directly editable before approval. `PeopleScreen` provides selection checkboxes, floating bulk action bar, and a dedicated entity merge modal with canonical name customization (`POST /api/people/merge`).

## Per-Statement Color-Coded Verification & Widescreen Architecture
- Per-Statement Evaluation & Summary Footer: `prompts/verify_v1.txt` and `crates/pipeline/src/verify.rs` (`statement_checks` on `RawVerificationLlmOutput`) evaluate each distinct factual sub-statement separately (`SUPPORTED` / `CONTRADICTED` / `UNVERIFIABLE`) and serialize structured `[ПОТВЪРДЕНО] / [ОПРОВЕРГАНО] / [НЕПРОВЕРИМО] <statement> :: <check>` records followed by `[ОБОБЩЕНИЕ] <summary>`. `DetailDrawer.svelte` parses both structured tags and legacy prose `reasoning_bg` into color-coded statement cards with clickable `[p1]` passage deep-links and a concluding summary footer.
- Calibrated Display Presets (`13"`–`18"`, `24"+`, `Auto`, `Single`): Tauri desktop (`ui/src-tauri/src/main.rs`) auto-maximizes on `aspect >= 1.55` displays (including `16:10` 14", 16", and 18" panels) and exposes `get_display_hardware_info` (physical resolution, OS DPI scale, logical dimensions, aspect ratio, EDID monitor name). Reactive `ui/src/lib/widescreen.svelte.ts` (`DISPLAY_PRESETS` + `widescreen`) auto-detects screen diagonal or applies manual presets from `TopBar.svelte` and `SettingsScreen.svelte`, syncing CSS custom properties (`--ui-font-size`, `--drawer-docked-width`, `--drawer-left-col`, `--video-stage-height`, `--people-card-min`, `--dossier-max-w`, `--dossier-toc-w`, `--evidence-card-min`, `--statement-card-min`) with DPI-compensated `18" ★` 3-pane command-desk geometry (`16:10` `2560×1600` / `3840×2400` at 100%–150% DPI).

## Fact-Dense Subject Reports, Ongoing Notifications, Book-Reader Navigation & Institutional Bulgarian
- Expanded Subject Taxonomy (`infrastructure`, `building`): Extraction (`prompts/claims_v1.txt`, `crates/pipeline/src/claims.rs`), heuristics (`detect_entity_type` in `crates/providers/src/people.rs`), archival checks (`web_archive.rs`), OSINT queries (`server.rs`), and UI (`PeopleScreen.svelte`, `PersonDossierModal.svelte`, `DetailDrawer.svelte`) support 8 entity types including `infrastructure` (highways, bridges, dams, power plants) and `building` (hospitals, schools, monuments, complexes).
- Zero-Boilerplate Fact-Dense Chapters & Deterministic Concrete-Fact Guarantees: `handle_research_person` (`crates/pipeline/src/server.rs`) harvests local verified video claims/evidence, executes short `OR`-grouped multi-bucket searches (`fin`, `net`, `gov`, `legal`, `media`) + 2nd-degree `pivot` queries, auto-simplifies zero-hit queries with Google News RSS fallback (`crates/providers/src/search.rs`), routes via `LlmChainRunner::from_role("osint_dossier")` (sequential on single-slot `local`, concurrent on cloud, direct deterministic synthesis on `heuristics`), and runs `ensure_concrete_facts_preserved_block1`/`block2` so discovered ЕИК numbers, company pivots, and sourced financial/legal facts are never omitted even if an LLM summarizes vaguely. Video claim extraction (`crates/pipeline/src/claims.rs` & `crates/providers/src/llm.rs`) includes `extract_concrete_claims_fallback` and section-aware `heuristic_extract_claims_json` so concrete factual statements with timestamps are always extracted even when an LLM is offline or returns empty claims.
- Live Ongoing Research Notifications: `PipelineContext.active_tasks` (`crates/pipeline/src/engine.rs`) + `/api/notifications` (`server.rs`) and `aiTasks.svelte.ts` expose live `ongoing` research tasks with progress bars and elapsed timers in `TopBar.svelte` and `NotificationCenterModal.svelte`, transitioning cleanly into clickable completed report/video notifications.
- Book-Reader `Съдържание` & Keyword Highlighting: `BookReaderToc.svelte` provides scroll-spy chapter navigation (`sidebar` and `compact` variants) across `PersonDossierModal.svelte`, `DetailDrawer.svelte` (sections, `Т1..Тn` claims, and article previews), `HelpScreen.svelte`, and `CalibrationScreen.svelte`. `ui/src/lib/reportHighlighter.ts` highlights dates in green (`.hl-date`), registry/case IDs (`.hl-id`), monetary amounts (`.hl-money`), and companies/oversight bodies (`.hl-entity`).
- Institutional Bulgarian Tone: Strictly sober, neutral institutional/editorial Bulgarian (`Аналитична справка` instead of `OSINT Разузнавателно Досие`); zero spy-movie AI clichés or literal English calques (`Магьосник`, `Фин скенер`, `Флагнати`, `Присъди`, `Инспектирай`).
- Build/test success: check Finished/test result lines; PowerShell exit code 1 from cargo stderr is not a failure.
